//! The canvas driven headlessly with synthetic pointer events: drawing, moving (one undo step), resizing by a
//! handle, marquee selection, and a click on a window over the canvas doing nothing.

use egui::{pos2, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};
use laserforge_app::canvas::{Canvas, Env};
use laserforge_app::prefs::Prefs;
use laserforge_app::project::Doc;
use laserforge_app::shapes::Shape;

struct Rig {
    ctx: egui::Context,
    canvas: Canvas,
    doc: Doc,
    prefs: Prefs,
    window: bool,
}

impl Rig {
    fn new() -> Rig {
        let mut canvas = Canvas::default();
        canvas.handlers = laserforge_app::tools::handlers();
        let mut prefs = Prefs::default();
        prefs.obj_snap = false;
        prefs.grid_snap = false;
        let mut r = Rig { ctx: egui::Context::default(), canvas, doc: Doc::default(), prefs, window: false };
        for _ in 0..5 {
            r.frame(vec![]);
        }
        r
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))), events, ..Default::default() };
        let layers = self.doc.layers.clone();
        let (canvas, doc, prefs, window) = (&mut self.canvas, &mut self.doc, &self.prefs, self.window);
        let mut out = self.ctx.run_ui(input, |ui| {
            let env = Env { layers: &layers, ws: (400.0, 400.0), head: None, prefs, snap_on: false, lock_aspect: true, filled: false, grid: true,
                            rulers: true, polygon_sides: 6, dark: true, modal: false, overlay: None };
            egui::CentralPanel::default().show_inside(ui, |ui| canvas.ui(ui, doc, &env));
            if window {
                egui::Window::new("over").fixed_pos(pos2(300.0, 300.0)).fixed_size(egui::vec2(300.0, 200.0)).show(ui.ctx(), |ui| {
                    ui.label("a window over the canvas");
                });
            }
        });
        out.textures_delta.clear();
    }

    fn at(&self, x: f64, y: f64) -> Pos2 {
        self.canvas.to_screen([x, y])
    }

    /// Press at world a, drag through to world b, release.
    fn drag(&mut self, a: [f64; 2], b: [f64; 2]) {
        let (pa, pb) = (self.at(a[0], a[1]), self.at(b[0], b[1]));
        let m = Modifiers::NONE;
        self.frame(vec![Event::PointerMoved(pa)]);
        self.frame(vec![Event::PointerButton { pos: pa, button: PointerButton::Primary, pressed: true, modifiers: m }]);
        for k in 1..=5 {
            let t = k as f32 / 5.0;
            self.frame(vec![Event::PointerMoved(pa + (pb - pa) * t)]);
        }
        self.frame(vec![Event::PointerButton { pos: pb, button: PointerButton::Primary, pressed: false, modifiers: m }]);
        self.frame(vec![]);
    }
}

#[test]
fn draw_move_resize_marquee() {
    let mut r = Rig::new();
    assert!(r.canvas.zoom > 0.5, "the view fits the work area");
    // draw a rectangle
    r.canvas.set_tool("rect");
    r.drag([50.0, 60.0], [150.0, 120.0]);
    assert_eq!(r.doc.shapes.len(), 1, "one rectangle drawn");
    let s = &r.doc.shapes[0];
    assert_eq!(s.kind(), "rect");
    let b = s.bounds();
    assert!((b[0] - 50.0).abs() < 1.0 && (b[1] - 60.0).abs() < 1.0 && (b[2] - 100.0).abs() < 1.0 && (b[3] - 60.0).abs() < 1.0, "{b:?}");
    let undo0 = r.doc.undo.len();
    // move it by grabbing its left edge
    r.canvas.set_tool("select");
    r.drag([b[0], b[1] + 12.0], [b[0] + 20.0, b[1] + 22.0]);
    let b2 = r.doc.shapes[0].bounds();
    assert!((b2[0] - b[0] - 20.0).abs() < 1.0 && (b2[1] - b[1] - 10.0).abs() < 1.0, "moved: {b2:?}");
    assert_eq!(r.doc.undo.len(), undo0 + 1, "one undo step for the move");
    // resize from the bottom-right handle
    let (x1, y1) = (b2[0] + b2[2], b2[1] + b2[3]);
    r.drag([x1, y1], [x1 + 50.0, y1 + 30.0]);
    let b3 = r.doc.shapes[0].bounds();
    assert!(b3[2] > b2[2] + 30.0, "resized: {b3:?} from {b2:?}");
    r.doc.undo();
    let b4 = r.doc.shapes[0].bounds();
    assert!((b4[2] - b2[2]).abs() < 1e-6, "undo restores the size");
    // marquee selects shapes fully inside the box (left to right)
    r.doc.shapes.push(Shape::circle(50, 0, 300.0, 300.0, 10.0, 10.0));
    r.doc.set_selection(vec![]);
    r.drag([280.0, 280.0], [330.0, 330.0]);
    assert_eq!(r.doc.selected, vec![50], "marquee selected the circle");
}

#[test]
fn click_on_window_over_canvas_does_nothing() {
    let mut r = Rig::new();
    r.window = true;
    r.frame(vec![]);
    r.frame(vec![]);
    r.canvas.set_tool("rect");
    // drag entirely inside the window placed at screen (300..600, 300..500)
    let a = r.canvas.to_world(pos2(350.0, 350.0));
    let b = r.canvas.to_world(pos2(450.0, 420.0));
    r.drag(a, b);
    assert!(r.doc.shapes.is_empty(), "no rectangle drawn under the window");
}

#[test]
fn true_size_zoom_uses_a_plausible_screen_density() {
    let v = laserforge_app::canvas::Canvas::real_px_per_mm();
    println!("points per mm: {v}");
    assert!((2.0..10.0).contains(&v));
}
