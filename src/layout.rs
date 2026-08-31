use std::cmp::Reverse;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutTier {
    #[default]
    Normal,
    Compact,
    Tiny,
    Overflow,
}

impl LayoutTier {
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Normal => "layout-normal",
            Self::Compact => "layout-compact",
            Self::Tiny => "layout-tiny",
            Self::Overflow => "layout-overflow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthProfile {
    pub normal_px: u16,
    pub compact_px: u16,
    pub tiny_px: u16,
    pub can_overflow: bool,
}

impl WidthProfile {
    pub const fn new(normal_px: u16, compact_px: u16, tiny_px: u16) -> Self {
        Self {
            normal_px,
            compact_px,
            tiny_px,
            can_overflow: true,
        }
    }

    pub const fn fixed_normal(self) -> Self {
        Self {
            normal_px: self.normal_px,
            compact_px: self.normal_px,
            tiny_px: self.normal_px,
            can_overflow: self.can_overflow,
        }
    }

    pub const fn width(self, tier: LayoutTier) -> u16 {
        match tier {
            LayoutTier::Normal => self.normal_px,
            LayoutTier::Compact => self.compact_px,
            LayoutTier::Tiny => self.tiny_px,
            LayoutTier::Overflow => 0,
        }
    }
}

pub fn module_width_profile(id: &str) -> WidthProfile {
    match id {
        "disk" => WidthProfile::new(210, 128, 76),
        "network" | "netttl" | "diskio" => WidthProfile::new(184, 128, 84),
        "media" | "wifi" | "cpumodel" | "host" | "os" | "kernel" => WidthProfile::new(210, 138, 86),
        "volume" | "brightness" => WidthProfile::new(126, 98, 72),
        "cpu_graph" | "ram_graph" | "net_graph" => WidthProfile::new(76, 58, 42),
        "clock" | "date" => WidthProfile::new(104, 78, 58),
        _ => WidthProfile::new(112, 84, 60),
    }
}

pub fn custom_width_profile() -> WidthProfile {
    WidthProfile::new(180, 116, 72)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutItem {
    pub key: String,
    pub priority: i32,
    pub order: usize,
    pub profile: WidthProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutDecision {
    pub key: String,
    pub tier: LayoutTier,
    pub width_px: u16,
}

fn consumed_width(items: &[LayoutItem], tiers: &[LayoutTier], gap_px: u16) -> u32 {
    let mut visible = 0_u32;
    let mut widths = 0_u32;
    for (item, tier) in items.iter().zip(tiers) {
        if *tier != LayoutTier::Overflow {
            visible += 1;
            widths = widths.saturating_add(u32::from(item.profile.width(*tier)));
        }
    }
    widths.saturating_add(u32::from(gap_px).saturating_mul(visible.saturating_sub(1)))
}

pub fn allocate_layout(
    items: &[LayoutItem],
    available_px: u32,
    gap_px: u16,
) -> Vec<LayoutDecision> {
    let mut tiers = vec![LayoutTier::Normal; items.len()];
    if consumed_width(items, &tiers, gap_px) <= available_px {
        return decisions(items, &tiers);
    }

    let mut candidates: Vec<usize> = (0..items.len()).collect();
    candidates.sort_by_key(|index| (items[*index].priority, Reverse(items[*index].order)));

    for target in [LayoutTier::Compact, LayoutTier::Tiny] {
        for &index in &candidates {
            let item = &items[index];
            if item.profile.width(target) >= item.profile.width(tiers[index]) {
                continue;
            }
            tiers[index] = target;
            if consumed_width(items, &tiers, gap_px) <= available_px {
                return decisions(items, &tiers);
            }
        }
    }

    for &index in &candidates {
        if !items[index].profile.can_overflow {
            continue;
        }
        tiers[index] = LayoutTier::Overflow;
        if consumed_width(items, &tiers, gap_px) <= available_px {
            break;
        }
    }
    decisions(items, &tiers)
}

fn decisions(items: &[LayoutItem], tiers: &[LayoutTier]) -> Vec<LayoutDecision> {
    items
        .iter()
        .zip(tiers)
        .map(|(item, tier)| LayoutDecision {
            key: item.key.clone(),
            tier: *tier,
            width_px: item.profile.width(*tier),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, priority: i32, order: usize) -> LayoutItem {
        LayoutItem {
            key: key.into(),
            priority,
            order,
            profile: WidthProfile::new(100, 70, 40),
        }
    }

    #[test]
    fn allocator_keeps_normal_tier_when_everything_fits() {
        let result = allocate_layout(&[item("a", 10, 0), item("b", 90, 1)], 204, 4);
        assert!(result
            .iter()
            .all(|decision| decision.tier == LayoutTier::Normal));
    }

    #[test]
    fn allocator_downgrades_lower_priority_first() {
        let result = allocate_layout(&[item("low", 10, 0), item("high", 90, 1)], 174, 4);
        assert_eq!(result[0].tier, LayoutTier::Compact);
        assert_eq!(result[1].tier, LayoutTier::Normal);
    }

    #[test]
    fn allocator_uses_tiny_before_overflow() {
        let result = allocate_layout(&[item("low", 10, 0), item("high", 90, 1)], 84, 4);
        assert_eq!(result[0].tier, LayoutTier::Tiny);
        assert_eq!(result[1].tier, LayoutTier::Tiny);
    }

    #[test]
    fn allocator_overflows_low_priority_deterministically() {
        let result = allocate_layout(&[item("low", 10, 0), item("high", 90, 1)], 70, 4);
        assert_eq!(result[0].tier, LayoutTier::Overflow);
        assert_eq!(result[1].tier, LayoutTier::Tiny);
    }

    #[test]
    fn newer_equal_priority_item_yields_first() {
        let result = allocate_layout(&[item("first", 50, 0), item("second", 50, 1)], 174, 4);
        assert_eq!(result[0].tier, LayoutTier::Normal);
        assert_eq!(result[1].tier, LayoutTier::Compact);
    }

    #[test]
    fn fixed_profile_skips_compact_and_tiny() {
        let mut fixed = item("fixed", 10, 0);
        fixed.profile = fixed.profile.fixed_normal();
        let result = allocate_layout(&[fixed, item("flex", 90, 1)], 144, 4);
        assert_eq!(result[0].tier, LayoutTier::Normal);
        assert_eq!(result[1].tier, LayoutTier::Tiny);
    }
}
