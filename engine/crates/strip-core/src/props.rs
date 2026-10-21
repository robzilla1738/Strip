use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use crate::layout::{crosses_display, layout, overlap, Visibility};
use crate::model::{Strip, Window, WindowId};
use crate::params::{DisplayFrame, LayoutParams, Viewport};

fn visible() -> Viewport {
    Viewport { x: 0.0, y: 0.0, width: 1440.0, height: 900.0 }
}

fn params_for(gap_outer: f64) -> LayoutParams {
    LayoutParams {
        gap_outer,
        gap_inner: 8.0,
        ..LayoutParams::standard()
    }
}

fn strip_from_ratios(ratios: &[f64]) -> Strip {
    let params = params_for(8.0);
    let mut strip = Strip::new();
    for (index, ratio) in ratios.iter().enumerate() {
        strip.insert(Window::new(index as u64 + 1, 1), 1, None, &params, visible());
        let last = strip.columns.len() - 1;
        strip.columns[last].width_ratio = *ratio;
    }
    strip
}

fn ratio_map(strip: &Strip) -> BTreeMap<u64, f64> {
    strip.width_ratios().into_iter().map(|(id, ratio)| (id.0, ratio)).collect()
}

fn window_ids(strip: &Strip) -> Vec<WindowId> {
    strip
        .columns
        .iter()
        .chain(strip.pinned_left.iter().map(|pin| &pin.column))
        .chain(strip.pinned_right.iter().map(|pin| &pin.column))
        .flat_map(|column| column.windows.iter().map(|window| window.id))
        .collect()
}

fn assert_same_ratios(before: &BTreeMap<u64, f64>, after: &BTreeMap<u64, f64>) -> Result<(), TestCaseError> {
    for (id, ratio) in before {
        prop_assert_eq!(after.get(id), Some(ratio));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_insert_remove_move_keeps_other_width_ratios(
        ratios in prop::collection::vec(0.25f64..0.9, 1..5),
        from in 0usize..5,
        to in 0usize..5,
        remove_at in 0usize..5,
    ) {
        let params = params_for(8.0);
        let mut strip = strip_from_ratios(&ratios);
        let before = ratio_map(&strip);

        strip.move_column(from % strip.columns.len(), to % strip.columns.len());
        assert_same_ratios(&before, &ratio_map(&strip))?;

        strip.move_left(&params, visible());
        strip.move_right(&params, visible());
        assert_same_ratios(&before, &ratio_map(&strip))?;

        let victim = strip.columns[remove_at % strip.columns.len()].windows[0].id;
        strip.remove(victim, &params, visible());
        let after_remove = ratio_map(&strip);
        for (id, ratio) in &before {
            if let Some(found) = after_remove.get(id) {
                prop_assert_eq!(found, ratio);
            }
        }

        let inserted_before = ratio_map(&strip);
        strip.insert(Window::new(9_000, 1), 1, None, &params, visible());
        assert_same_ratios(&inserted_before, &ratio_map(&strip))?;
    }

    #[test]
    fn prop_every_window_in_exactly_one_column(
        ratios in prop::collection::vec(0.3f64..0.7, 1..4),
        extra in prop::collection::vec(1u64..30, 0..8),
    ) {
        let params = params_for(8.0);
        let mut strip = strip_from_ratios(&ratios);
        let mut live: BTreeSet<u64> = (1..=ratios.len() as u64).collect();
        for id in extra {
            if live.insert(id) {
                strip.insert(Window::new(id, 2), 2, None, &params, visible());
            }
        }
        if !live.is_empty() {
            let remove_id = *live.iter().next().unwrap();
            strip.remove(WindowId(remove_id), &params, visible());
            live.remove(&remove_id);
        }
        let ids = window_ids(&strip);
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        prop_assert_eq!(ids.len(), unique.len());
        prop_assert_eq!(unique.len(), live.len());
    }

    #[test]
    fn prop_frames_respect_display_edges(
        count in 1usize..6,
        scroll in -400.0f64..2200.0,
        gap_outer in prop::sample::select(vec![0.0, 8.0]),
        tabbed in proptest::bool::ANY,
    ) {
        let params = params_for(gap_outer);
        let mut ratios = vec![0.5; count];
        ratios[0] = 0.35;
        let mut strip = strip_from_ratios(&ratios);
        strip.scroll_x = scroll;
        if tabbed {
            if let Some(column) = strip.columns.first_mut() {
                column.mode = crate::model::DisplayMode::Tabbed;
                column.windows.push(Window::new(50, 1));
                column.active_tab = 0;
            }
        }
        let display = DisplayFrame::from_origin(1440.0, 900.0);
        let placed = layout(&strip, visible(), display, &params);
        let sliver = if gap_outer == 0.0 { 1.0 } else { params.sliver };
        for item in &placed {
            match item.visibility {
                Visibility::Visible => prop_assert!(!crosses_display(item.frame, display)),
                Visibility::Partial => {
                    let (width, _) = overlap(item.frame, display);
                    prop_assert!(width + 1e-6 >= sliver);
                }
                Visibility::Hidden => {}
            }
        }
    }

    #[test]
    fn prop_hidden_windows_expose_at_most_sliver(
        count in 2usize..7,
        scroll in -200.0f64..3000.0,
        gap_outer in prop::sample::select(vec![0.0, 8.0]),
        neighbours in prop::sample::select(vec![(false, false), (true, false), (false, true), (true, true)]),
    ) {
        let params = params_for(gap_outer);
        let mut strip = strip_from_ratios(&vec![0.55; count]);
        strip.scroll_x = scroll;
        let mut display = DisplayFrame::from_origin(1440.0, 900.0);
        display.neighbour_left = neighbours.0;
        display.neighbour_right = neighbours.1;
        let allowed = if neighbours.0 && neighbours.1 {
            1.0
        } else if gap_outer == 0.0 {
            1.0
        } else {
            params.sliver
        };
        let placed = layout(&strip, visible(), display, &params);
        for item in placed {
            if item.visibility != Visibility::Hidden {
                continue;
            }
            let (width, _) = overlap(item.frame, display);
            prop_assert!(width <= allowed + 1e-4);
            prop_assert!(item.frame.y + 1e-4 >= display.min_y);
            prop_assert!(item.frame.bottom() <= display.max_y + 1e-4);
        }
    }
}
