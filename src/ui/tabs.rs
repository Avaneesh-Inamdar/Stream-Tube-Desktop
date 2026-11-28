//! Browser-style tab layout, tab groups, drag-and-drop payloads, and persistence.
use crate::i18n::t;
use crate::store;
use crate::ui::app::Route;
use crate::ui::{icon, pal};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabBarPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl TabBarPosition {
    pub const CHOICES: &[(&str, &str, &str)] = &[
        ("top", "Top", "panel-top"),
        ("bottom", "Bottom", "panel-bottom"),
        ("left", "Left", "panel-left"),
        ("right", "Right", "panel-right"),
    ];

    pub fn parse(value: &str) -> Self {
        match value {
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Top,
        }
    }

    pub fn normalize(value: &str) -> &'static str {
        Self::parse(value).as_str()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

pub const GROUP_COLORS: &[(u32, &str)] = &[
    (0x3ea6ff, "Blue"),
    (0xab47bc, "Purple"),
    (0xf0307e, "Pink"),
    (0xff4e45, "Red"),
    (0xffa000, "Amber"),
    (0x2ba640, "Green"),
    (0x00bfa5, "Teal"),
    (0x8e8e93, "Gray"),
];

pub fn next_group_color(groups: &[TabGroup]) -> u32 {
    for &(color, _) in GROUP_COLORS {
        if !groups.iter().any(|g| g.color == color) {
            return color;
        }
    }
    GROUP_COLORS[groups.len() % GROUP_COLORS.len()].0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    pub id: u64,
    pub name: String,
    pub color: u32,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub route: Route,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub group_id: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedTabLayout {
    #[serde(default)]
    pub groups: Vec<TabGroup>,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active_index: usize,
}

pub fn load_layout() -> Option<SavedTabLayout> {
    store::kv_get("tab_layout")
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str::<SavedTabLayout>(&s).ok())
}

pub fn save_layout(layout: &SavedTabLayout) {
    if let Ok(json) = serde_json::to_string(layout) {
        store::kv_set("tab_layout", &json);
    }
}

/// Trait allowing layout algorithms to operate cleanly on both `AppView::tabs` and unit test stubs.
pub trait TabItem {
    fn tab_id(&self) -> u64;
    fn group_id(&self) -> Option<u64>;
    fn set_group_id(&mut self, group: Option<u64>);
}

/// Ensures tabs belonging to the same group stay contiguous in the strip.
pub fn normalize_group_contiguity<T: TabItem>(tabs: &mut Vec<T>) {
    let mut seen_groups: HashSet<u64> = HashSet::new();
    let mut out: Vec<T> = Vec::with_capacity(tabs.len());
    let mut remaining: Vec<Option<T>> = tabs.drain(..).map(Some).collect();

    for idx in 0..remaining.len() {
        let Some(item) = remaining[idx].take() else {
            continue;
        };
        let gid = item.group_id();
        out.push(item);
        if let Some(g) = gid {
            if seen_groups.insert(g) {
                for other_idx in (idx + 1)..remaining.len() {
                    if remaining[other_idx].as_ref().is_some_and(|t| t.group_id() == Some(g)) {
                        if let Some(other) = remaining[other_idx].take() {
                            out.push(other);
                        }
                    }
                }
            }
        }
    }
    *tabs = out;
}

/// Reorders `dragged_id` next to `target_id` (before or after depending on relative order)
/// and adopts `target_id`'s group membership.
pub fn move_tab_onto_tab<T: TabItem>(tabs: &mut Vec<T>, dragged_id: u64, target_id: u64) -> bool {
    if dragged_id == target_id {
        return false;
    }
    let Some(src) = tabs.iter().position(|t| t.tab_id() == dragged_id) else {
        return false;
    };
    let Some(dst) = tabs.iter().position(|t| t.tab_id() == target_id) else {
        return false;
    };
    let after = src < dst;
    let target_group = tabs[dst].group_id();
    let mut item = tabs.remove(src);
    item.set_group_id(target_group);
    let Some(new_dst) = tabs.iter().position(|t| t.tab_id() == target_id) else {
        tabs.push(item);
        return true;
    };
    let insert_at = if after { new_dst + 1 } else { new_dst };
    tabs.insert(insert_at, item);
    normalize_group_contiguity(tabs);
    true
}

/// Moves `tab_id` into `group_id` (at the end of that group's block) or removes it from its group (`None`).
pub fn move_tab_to_group<T: TabItem>(tabs: &mut Vec<T>, tab_id: u64, group_id: Option<u64>) -> bool {
    let Some(src) = tabs.iter().position(|t| t.tab_id() == tab_id) else {
        return false;
    };
    let old_group = tabs[src].group_id();
    if old_group == group_id {
        return false;
    }
    let mut item = tabs.remove(src);
    item.set_group_id(group_id);
    match group_id {
        Some(gid) => {
            if let Some(last) = tabs.iter().rposition(|t| t.group_id() == Some(gid)) {
                tabs.insert(last + 1, item);
            } else {
                tabs.push(item);
            }
        }
        None => {
            if let Some(prev_gid) = old_group {
                if let Some(last) = tabs.iter().rposition(|t| t.group_id() == Some(prev_gid)) {
                    tabs.insert(last + 1, item);
                } else {
                    tabs.insert(src.min(tabs.len()), item);
                }
            } else {
                tabs.push(item);
            }
        }
    }
    normalize_group_contiguity(tabs);
    true
}

/// Moves `tab_id` to the end of the tab strip as an ungrouped tab.
pub fn move_tab_to_ungrouped_end<T: TabItem>(tabs: &mut Vec<T>, tab_id: u64) -> bool {
    let Some(src) = tabs.iter().position(|t| t.tab_id() == tab_id) else {
        return false;
    };
    if src + 1 == tabs.len() && tabs[src].group_id().is_none() {
        return false;
    }
    let mut item = tabs.remove(src);
    item.set_group_id(None);
    tabs.push(item);
    normalize_group_contiguity(tabs);
    true
}

/// Data carried while dragging a tab across the tab bar.
#[derive(Clone, Debug)]
pub struct DragTab {
    pub id: u64,
    pub index: usize,
    pub label: String,
    pub icon: &'static str,
    pub group_color: Option<u32>,
}

/// Floating pill shown under the pointer while dragging a tab.
pub struct DragTabPreview {
    pub drag: DragTab,
}

impl Render for DragTabPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let label = if self.drag.label.is_empty() {
            t("Tab").to_string()
        } else {
            self.drag.label.clone()
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .h(px(30.))
            .max_w(px(210.))
            .rounded_lg()
            .bg(p.surface2)
            .border_1()
            .border_color(self.drag.group_color.map(|c| rgb(c).into()).unwrap_or(p.accent))
            .shadow_lg()
            .opacity(0.92)
            .when_some(self.drag.group_color, |d, c| {
                d.child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(c)))
            })
            .child(icon(self.drag.icon).size_4().text_color(p.accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text)
                    .child(label),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct StubTab {
        id: u64,
        group: Option<u64>,
    }

    impl TabItem for StubTab {
        fn tab_id(&self) -> u64 {
            self.id
        }
        fn group_id(&self) -> Option<u64> {
            self.group
        }
        fn set_group_id(&mut self, group: Option<u64>) {
            self.group = group;
        }
    }

    #[::core::prelude::v1::test]
    fn reorder_and_cross_group_drag_works() {
        let mut tabs = vec![
            StubTab { id: 1, group: Some(10) },
            StubTab { id: 2, group: Some(10) },
            StubTab { id: 3, group: None },
            StubTab { id: 4, group: Some(20) },
        ];

        // Drag tab 3 onto tab 1 (leftward): inserts before tab 1 and joins group 10
        assert!(move_tab_onto_tab(&mut tabs, 3, 1));
        assert_eq!(
            tabs,
            vec![
                StubTab { id: 3, group: Some(10) },
                StubTab { id: 1, group: Some(10) },
                StubTab { id: 2, group: Some(10) },
                StubTab { id: 4, group: Some(20) },
            ]
        );

        // Drag tab 3 onto tab 4 (rightward): moves into group 20 after tab 4
        assert!(move_tab_onto_tab(&mut tabs, 3, 4));
        assert_eq!(
            tabs,
            vec![
                StubTab { id: 1, group: Some(10) },
                StubTab { id: 2, group: Some(10) },
                StubTab { id: 4, group: Some(20) },
                StubTab { id: 3, group: Some(20) },
            ]
        );
    }

    #[::core::prelude::v1::test]
    fn move_to_group_and_ungroup_keeps_contiguity() {
        let mut tabs = vec![
            StubTab { id: 1, group: Some(10) },
            StubTab { id: 2, group: Some(10) },
            StubTab { id: 3, group: Some(10) },
            StubTab { id: 4, group: None },
        ];

        // Ungroup middle tab 2 -> moves right after group 10 so group stays contiguous
        assert!(move_tab_to_group(&mut tabs, 2, None));
        assert_eq!(
            tabs,
            vec![
                StubTab { id: 1, group: Some(10) },
                StubTab { id: 3, group: Some(10) },
                StubTab { id: 2, group: None },
                StubTab { id: 4, group: None },
            ]
        );

        // Move tab 4 into group 10 -> lands at end of group 10
        assert!(move_tab_to_group(&mut tabs, 4, Some(10)));
        assert_eq!(
            tabs,
            vec![
                StubTab { id: 1, group: Some(10) },
                StubTab { id: 3, group: Some(10) },
                StubTab { id: 4, group: Some(10) },
                StubTab { id: 2, group: None },
            ]
        );
    }

    #[::core::prelude::v1::test]
    fn tab_bar_position_round_trip() {
        for (raw, expected) in [
            ("top", TabBarPosition::Top),
            ("bottom", TabBarPosition::Bottom),
            ("left", TabBarPosition::Left),
            ("right", TabBarPosition::Right),
            ("invalid", TabBarPosition::Top),
        ] {
            assert_eq!(TabBarPosition::parse(raw), expected);
        }
        assert!(TabBarPosition::Left.is_vertical());
        assert!(TabBarPosition::Right.is_vertical());
        assert!(!TabBarPosition::Top.is_vertical());
        assert!(!TabBarPosition::Bottom.is_vertical());
    }
}
