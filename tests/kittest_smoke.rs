//! Smoke test: `egui_kittest::Harness` boots against a no-op `egui` app,
//! runs one frame, and exercises the AccessKit query path via
//! `get_by_label`. This is the contract that every kittest test in this
//! crate relies on; if it fails, no kittest test can pass.

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

#[test]
fn harness_runs_one_frame_against_a_no_op_app() {
    let mut harness = Harness::new(|ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("hello kittest");
        });
    });
    harness.run();
    // `get_by_label` panics on miss in egui_kittest 0.31.1, so reaching
    // the next line without panic IS the assertion that the AccessKit
    // query path found the labelled node.
    let _node = harness.get_by_label("hello kittest");
}
