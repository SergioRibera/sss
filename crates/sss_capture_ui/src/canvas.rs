//! Canvas state — the editable model behind the overlay.
//!
//! The canvas is a flat list of [`Shape`]s in z-order (back to front), plus
//! a small history stack for undo / redo. Every input event from the
//! platform layer turns into a [`CanvasEvent`] and feeds [`Canvas::handle`].

use sss_capture::Rect;

use crate::geometry::{FPoint, FRect};
use crate::shape::{Shape, ShapeId, ShapeKind, Style};
use crate::tool::{StepSettings, Tool};

/// Logical input event the canvas understands. Platform code translates
/// raw pointer / keyboard events into these.
#[derive(Clone, Debug)]
pub enum CanvasEvent {
    PointerDown(FPoint),
    PointerMove(FPoint),
    PointerUp(FPoint),
    /// Cancel any in-flight drag without committing.
    PointerCancel,
    /// Type a character (Text tool).
    TextInput(char),
    /// Backspace while editing text.
    TextBackspace,
    /// Commit the in-flight text shape.
    TextCommit,
    /// Discard the in-flight text shape (don't commit it).
    TextCancel,
    Undo,
    Redo,
    /// Delete the currently selected shape.
    Delete,
}

#[derive(Clone, Debug)]
pub struct Canvas {
    next_id: u64,
    shapes: Vec<Shape>,
    /// Region rectangle the user is selecting (the rubber-band rect).
    region: Option<FRect>,
    /// Active tool — driven by the toolbar; the canvas uses it to know what
    /// to commit on PointerUp.
    pub active_tool: Tool,
    drag: Option<Drag>,
    selected: Option<ShapeId>,
    /// Next step number for the Step tool.
    next_step: u32,
    /// Currently-being-typed text shape, if any.
    pending_text: Option<PendingText>,
    history: History,
    /// When `true`, closed shapes committed by the user (rectangle / ellipse)
    /// get their interior filled. The fill colour defaults to the stroke
    /// colour but can be overridden by `fill_color` for a separate
    /// stroke / fill workflow.
    fill_mode: bool,
    /// Optional explicit fill colour for the next closed shape. When
    /// `None`, the current stroke colour is used. Set via Shift+click on
    /// a colour swatch (separate from the regular stroke colour).
    fill_color: Option<crate::color::Color>,
    /// In-flight polygon vertices. Active while the user is in the
    /// Polygon tool and has clicked at least once; consumed on
    /// `commit_polygon`.
    pending_polygon: Option<Vec<FPoint>>,
    /// "Constrain" modifier — Hold-Shift while dragging snaps lines to
    /// 45° increments and forces rectangles / ellipses to be squares /
    /// circles. The platform driver flips this from `wl_keyboard`
    /// modifiers; the canvas just reads it inside `on_move`.
    constrain: bool,
}

impl Canvas {
    pub fn new() -> Self {
        let mut s = Self {
            next_id: 1,
            shapes: Vec::new(),
            region: None,
            active_tool: Tool::Pointer,
            drag: None,
            selected: None,
            next_step: 1,
            pending_text: None,
            history: History::default(),
            fill_mode: false,
            fill_color: None,
            pending_polygon: None,
            constrain: false,
        };
        // Push an empty snapshot so Undo can roll us all the way back to a
        // clean slate. Without it, the *first* user action couldn't be
        // undone (the undo stack would pop the only saved state and have
        // nothing to restore from).
        s.history.snapshot(&s.shapes);
        s
    }

    /// Whether the next closed shape (rect / ellipse) should be filled.
    pub fn fill_mode(&self) -> bool {
        self.fill_mode
    }

    /// Toggle the fill mode. Affects shapes drawn *after* the toggle —
    /// existing shapes are untouched.
    pub fn toggle_fill_mode(&mut self) {
        self.fill_mode = !self.fill_mode;
    }

    /// Override the fill colour used while `fill_mode` is on. Pass
    /// `None` to turn fill off entirely (clears both `fill_color`
    /// *and* `fill_mode`).
    ///
    /// The "set None just clears the explicit colour, fill_mode stays
    /// on" interpretation was confusing in practice: clicking the
    /// outlined variant of a closed shape would clear the colour but
    /// the next stroke would still come out filled because
    /// `fill_mode == true`. Treating `None` as "fill off" matches the
    /// callers' actual intent.
    pub fn set_fill_color(&mut self, c: Option<crate::color::Color>) {
        self.fill_color = c;
        self.fill_mode = c.is_some();
    }

    /// Read the active fill colour (or `None` when "auto-from-stroke").
    pub fn fill_color(&self) -> Option<crate::color::Color> {
        self.fill_color
    }

    /// All committed shapes, back-to-front.
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Mutable iterator — used by the Pointer tool when moving / resizing.
    pub fn shapes_mut(&mut self) -> &mut Vec<Shape> {
        &mut self.shapes
    }

    /// The region rectangle, integer-rounded. `None` until the user has
    /// dragged at least once in `Area` mode.
    pub fn region(&self) -> Option<Rect> {
        self.region.map(FRect::to_int)
    }

    pub fn set_region(&mut self, r: Option<Rect>) {
        self.region = r.map(FRect::from);
    }

    /// Whether the user is currently holding the mouse button and dragging.
    /// Platform drivers use this to decide whether a `PointerMove` event
    /// needs a repaint (a hover-only motion changes nothing visible).
    pub fn is_drag_active(&self) -> bool {
        self.drag.is_some()
    }

    /// Anchor point for the currently active drag — the start of the
    /// two-point gesture, the rubber-band origin or the moved shape's
    /// drag-start cursor. Returns `None` when no drag is in progress or
    /// the drag kind doesn't have a natural anchor (eraser/freehand).
    /// Used by the platform driver to draw constrain reference guides.
    pub fn drag_anchor(&self) -> Option<FPoint> {
        match self.drag.as_ref()? {
            Drag::TwoPoint { from, .. } => Some(*from),
            Drag::Region { from, .. } => Some(*from),
            Drag::RegionMove { start, .. } => Some(*start),
            Drag::Move { start, .. } => Some(*start),
            Drag::Stroke { points } => points.first().copied(),
            Drag::RegionResize { .. } | Drag::Erase { .. } => None,
        }
    }

    /// Whether the user is in the middle of typing a text shape. Platform
    /// drivers route Enter / Escape to `TextCommit` / `TextCancel` instead
    /// of the global confirm / cancel while this is `true`.
    pub fn is_typing_text(&self) -> bool {
        self.pending_text.is_some()
    }

    /// Replace the active tool (toolbar callback).
    pub fn set_tool(&mut self, t: Tool) {
        self.cancel_drag();
        // When the user switches away from Text mid-edit, commit the pending
        // string so it doesn't get lost.
        if !matches!(t, Tool::Text(_)) {
            self.commit_pending_text();
        }
        // Same idea for Polygon: leaving the tool commits whatever vertices
        // the user already clicked, so they aren't silently discarded.
        if !matches!(t, Tool::Polygon(_)) && self.is_drawing_polygon() {
            self.commit_polygon();
        }
        self.active_tool = t;
    }

    pub fn selected(&self) -> Option<ShapeId> {
        self.selected
    }

    pub fn select(&mut self, id: Option<ShapeId>) {
        self.selected = id;
    }

    /// Push the current in-flight drag preview as a fresh shape. Used by the
    /// render path to peek at the work-in-progress.
    pub fn preview_shape(&self) -> Option<Shape> {
        let drag = self.drag.as_ref()?;
        let id = ShapeId(0); // preview-only; never inserted with this id
        let style = current_style_for_canvas(self);
        let kind = match (&self.active_tool, drag) {
            (Tool::Brush(_), Drag::Stroke { points, .. }) => ShapeKind::FreehandStroke {
                points: points.clone(),
            },
            (Tool::Line(_), Drag::TwoPoint { from, to, .. }) => ShapeKind::Line {
                from: *from,
                to: *to,
            },
            (Tool::Arrow(_), Drag::TwoPoint { from, to, .. }) => ShapeKind::Arrow {
                from: *from,
                to: *to,
            },
            (Tool::Rectangle(_), Drag::TwoPoint { from, to, .. }) => ShapeKind::Rectangle {
                rect: FRect::from_corners(*from, *to).to_int(),
            },
            (Tool::Ellipse(_), Drag::TwoPoint { from, to, .. }) => ShapeKind::Ellipse {
                rect: FRect::from_corners(*from, *to).to_int(),
            },
            (Tool::BlurRect { radius }, Drag::TwoPoint { from, to, .. }) => ShapeKind::BlurRect {
                rect: FRect::from_corners(*from, *to).to_int(),
                radius: *radius,
            },
            _ => return None,
        };
        Some(Shape {
            id,
            kind,
            style,
            rotation: 0.0,
        })
    }

    /// Render-time view of the pending text shape (so the user can see what
    /// they're typing before committing).
    pub fn pending_text(&self) -> Option<Shape> {
        self.pending_text.as_ref().map(|p| Shape {
            id: ShapeId(0),
            kind: ShapeKind::Text {
                origin: p.origin,
                content: p.text.clone(),
                style: p.style.clone(),
            },
            style: Style {
                stroke: p.style.color,
                stroke_width: 1.0,
                fill: None,
            },
            rotation: 0.0,
        })
    }

    /// Apply an input event.
    pub fn handle(&mut self, ev: CanvasEvent) {
        match ev {
            CanvasEvent::PointerDown(p) => self.on_down(p),
            CanvasEvent::PointerMove(p) => self.on_move(p),
            CanvasEvent::PointerUp(p) => self.on_up(p),
            CanvasEvent::PointerCancel => self.cancel_drag(),
            CanvasEvent::TextInput(c) => self.on_text_char(c),
            CanvasEvent::TextBackspace => self.on_text_backspace(),
            CanvasEvent::TextCommit => self.commit_pending_text(),
            CanvasEvent::TextCancel => {
                self.pending_text = None;
            }
            CanvasEvent::Undo => self.undo(),
            CanvasEvent::Redo => self.redo(),
            CanvasEvent::Delete => self.delete_selected(),
        }
    }

    // ----- event handlers --------------------------------------------------

    fn on_down(&mut self, p: FPoint) {
        match &self.active_tool {
            Tool::Pointer => {
                // Test top-to-bottom (last shape wins — it's on top in z-order).
                self.selected = self
                    .shapes
                    .iter()
                    .rev()
                    .find(|s| s.contains(p))
                    .map(|s| s.id);
                if let Some(sel) = self.selected {
                    let original = self
                        .shapes
                        .iter()
                        .find(|s| s.id == sel)
                        .cloned()
                        .unwrap_or_else(|| Shape {
                            id: sel,
                            kind: ShapeKind::FreehandStroke { points: vec![] },
                            style: Style::default(),
                            rotation: 0.0,
                        });
                    self.drag = Some(Drag::Move {
                        id: sel,
                        start: p,
                        original_shape: Box::new(original),
                    });
                } else if let Some(region) = self.region.map(FRect::to_int) {
                    // There's an existing region — figure out what the user
                    // wants to do with it based on where they clicked:
                    //   * On a corner / edge handle → resize that handle.
                    //   * Inside the region → move the whole rectangle.
                    //   * Outside the region → discard and start a fresh
                    //     rubber-band drag.
                    let handle = pointer_handle(&region, p);
                    match handle {
                        Some(h) => {
                            self.drag = Some(Drag::RegionResize {
                                handle: h,
                                original: region,
                            });
                        }
                        None if region_contains(&region, p) => {
                            self.drag = Some(Drag::RegionMove {
                                start: p,
                                original: region,
                            });
                        }
                        None => {
                            self.region = None;
                            self.drag = Some(Drag::Region { from: p, to: p });
                        }
                    }
                } else {
                    // First selection.
                    self.drag = Some(Drag::Region { from: p, to: p });
                }
            }
            Tool::Brush(_) => {
                self.drag = Some(Drag::Stroke { points: vec![p] });
            }
            Tool::Line(_) | Tool::Arrow(_) | Tool::Rectangle(_) | Tool::Ellipse(_) => {
                self.drag = Some(Drag::TwoPoint { from: p, to: p });
            }
            Tool::BlurRect { .. } => {
                self.drag = Some(Drag::TwoPoint { from: p, to: p });
            }
            Tool::Eraser { radius } => {
                let r = *radius;
                self.erase_at(p, r);
                self.drag = Some(Drag::Erase { radius: r });
            }
            Tool::Step(settings) => {
                let s = *settings;
                self.place_step(p, s);
            }
            Tool::Text(style) => {
                let style = style.clone();
                self.commit_pending_text();
                self.pending_text = Some(PendingText {
                    origin: p,
                    text: String::new(),
                    style,
                });
            }
            Tool::Polygon(_) => {
                // Each click appends a vertex; `on_up` is a no-op for this
                // tool. Press Enter (or right-click — see the platform
                // driver) to close + commit the polygon.
                let pending = self.pending_polygon.get_or_insert_with(Vec::new);
                pending.push(p);
            }
        }
    }

    /// Add the current pointer position as a *preview* polygon vertex.
    /// Used by the renderer to draw a hint segment from the last clicked
    /// vertex to the live cursor while the user is still building the
    /// polygon.
    pub fn polygon_preview_tip(&self) -> Option<FPoint> {
        // The actual preview is computed in `preview_shape`; this getter
        // is exposed so the live cursor + drag handling stay symmetrical
        // with the other tools.
        None
    }

    /// Commits the in-flight polygon as a closed shape. No-op if there are
    /// fewer than 2 vertices.
    pub fn commit_polygon(&mut self) {
        let pts = match self.pending_polygon.take() {
            Some(p) if p.len() >= 2 => p,
            _ => return,
        };
        let style = current_style_for_canvas(self);
        let id = self.alloc_id();
        self.push_shape(Shape {
            id,
            kind: ShapeKind::Polygon {
                points: pts,
                closed: true,
            },
            style,
            rotation: 0.0,
        });
    }

    /// Drops any in-flight polygon without committing.
    pub fn cancel_polygon(&mut self) {
        self.pending_polygon = None;
    }

    /// Is the user mid-way through a polygon?
    pub fn is_drawing_polygon(&self) -> bool {
        self.pending_polygon
            .as_ref()
            .map_or(false, |v| !v.is_empty())
    }

    /// In-flight polygon vertices (used by the renderer for the live preview).
    pub fn polygon_vertices(&self) -> Option<&[FPoint]> {
        self.pending_polygon.as_deref()
    }

    /// Style the renderer should use for the in-flight polygon preview.
    pub fn current_polygon_style(&self) -> Style {
        current_style_for_canvas(self)
    }

    /// Wipe every committed shape and reset the in-flight state. The
    /// region rectangle stays so the user doesn't lose their selection;
    /// undo can roll the clear back like any other action.
    pub fn clear_shapes(&mut self) {
        self.shapes.clear();
        self.selected = None;
        self.pending_text = None;
        self.pending_polygon = None;
        self.history.snapshot(&self.shapes);
    }

    /// Hold-Shift constraint flag. Toggled live by the platform driver
    /// from the keyboard modifier state, then read inside [`on_move`] to
    /// snap two-point drags to 45° / square / circle.
    pub fn set_constrain(&mut self, on: bool) {
        self.constrain = on;
    }

    /// Move the currently-selected shape one layer up (towards the front).
    pub fn raise_selected(&mut self) {
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        let idx = match self.shapes.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => return,
        };
        if idx + 1 < self.shapes.len() {
            self.shapes.swap(idx, idx + 1);
            self.history.snapshot(&self.shapes);
        }
    }

    /// Move the currently-selected shape one layer down (towards the back).
    pub fn lower_selected(&mut self) {
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        let idx = match self.shapes.iter().position(|s| s.id == id) {
            Some(i) => i,
            None => return,
        };
        if idx > 0 {
            self.shapes.swap(idx, idx - 1);
            self.history.snapshot(&self.shapes);
        }
    }

    /// Move the currently-selected shape all the way to the front (top of z-stack).
    pub fn raise_to_top(&mut self) {
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        if let Some(idx) = self.shapes.iter().position(|s| s.id == id) {
            let shape = self.shapes.remove(idx);
            self.shapes.push(shape);
            self.history.snapshot(&self.shapes);
        }
    }

    /// Uniformly scale the selected shape about its bounds centre. A factor
    /// of 1.0 is a no-op; values < 1 shrink, values > 1 grow. Strokes,
    /// polygons, rectangles, ellipses, lines/arrows and text are all
    /// supported; the Step tool already has its own radius and just gets
    /// its centre scaled too.
    pub fn scale_selected(&mut self, factor: f32) {
        if !factor.is_finite() || (factor - 1.0).abs() < f32::EPSILON {
            return;
        }
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        let Some(idx) = self.shapes.iter().position(|s| s.id == id) else {
            return;
        };
        let shape = &mut self.shapes[idx];
        let bounds = shape.bounds();
        let cx = bounds.x() as f32 + bounds.width() as f32 / 2.0;
        let cy = bounds.y() as f32 + bounds.height() as f32 / 2.0;
        scale_shape(shape, cx, cy, factor);
        self.history.snapshot(&self.shapes);
    }

    /// Rotate the selected shape by `radians` about its bounds centre.
    /// Only shapes that store explicit point lists (freehand, polygon,
    /// line, arrow) can rotate freely — rectangles and ellipses get
    /// converted to a tight axis-aligned bound after rotation, so a 90°
    /// rotate keeps them intact while intermediate angles approximate.
    pub fn rotate_selected(&mut self, radians: f32) {
        if !radians.is_finite() || radians.abs() < 1e-4 {
            return;
        }
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        let Some(idx) = self.shapes.iter().position(|s| s.id == id) else {
            return;
        };
        let shape = &mut self.shapes[idx];
        let bounds = shape.bounds();
        let cx = bounds.x() as f32 + bounds.width() as f32 / 2.0;
        let cy = bounds.y() as f32 + bounds.height() as f32 / 2.0;
        rotate_shape(shape, cx, cy, radians);
        self.history.snapshot(&self.shapes);
    }

    /// Overwrite the shape with the given id with `new_shape`, keeping
    /// its position in the z-stack. No-op when the id isn't found.
    /// Does *not* snapshot history — callers are expected to commit a
    /// snapshot at the end of an interactive drag.
    pub fn replace_shape(&mut self, id: ShapeId, new_shape: Shape) {
        if let Some(s) = self.shapes.iter_mut().find(|s| s.id == id) {
            *s = new_shape;
        }
    }

    /// Push a history snapshot of the current shape list. Used by
    /// drag-completion handlers in the platform driver.
    pub fn snapshot_history(&mut self) {
        self.history.snapshot(&self.shapes);
    }

    /// Move the currently-selected shape all the way to the back.
    pub fn lower_to_bottom(&mut self) {
        let id = match self.selected {
            Some(id) => id,
            None => return,
        };
        if let Some(idx) = self.shapes.iter().position(|s| s.id == id) {
            let shape = self.shapes.remove(idx);
            self.shapes.insert(0, shape);
            self.history.snapshot(&self.shapes);
        }
    }

    fn on_move(&mut self, p: FPoint) {
        match self.drag.as_mut() {
            Some(Drag::Stroke { points }) => points.push(p),
            Some(Drag::TwoPoint { from, to }) => {
                *to = if self.constrain {
                    apply_constraint(&self.active_tool, *from, p)
                } else {
                    p
                };
            }
            Some(Drag::Region { from, to }) => {
                *to = p;
                // Update the visible region live so the rubber-band shows
                // up while the user is still dragging it (instead of only
                // appearing once they let go).
                self.region = Some(FRect::from_corners(*from, *to));
            }
            Some(Drag::RegionMove { start, original }) => {
                let dx = (p.x - start.x).round() as i32;
                let dy = (p.y - start.y).round() as i32;
                let original = *original;
                let moved = Rect::from_xywh(
                    original.x() + dx,
                    original.y() + dy,
                    original.width(),
                    original.height(),
                );
                self.region = Some(FRect::from(moved));
            }
            Some(Drag::RegionResize { handle, original }) => {
                let original = *original;
                let dx = (p.x - (original.x() as f32 + handle_pivot_x(*handle, &original))).round()
                    as i32;
                let dy = (p.y - (original.y() as f32 + handle_pivot_y(*handle, &original))).round()
                    as i32;
                let resized = resize_region(*handle, original, dx, dy);
                self.region = Some(FRect::from(resized));
            }
            Some(Drag::Move {
                id,
                start,
                original_shape,
            }) => {
                let dx = (p.x - start.x).round() as i32;
                let dy = (p.y - start.y).round() as i32;
                let id = *id;
                let baseline: Shape = (**original_shape).clone();
                if let Some(shape) = self.shapes.iter_mut().find(|s| s.id == id) {
                    // Restore the shape to its drag-start state, *then*
                    // translate by the delta. This makes each motion event
                    // idempotent — without this, the previous code
                    // accumulated `dx`/`dy` across events and shapes flew
                    // away the moment the user nudged the mouse.
                    *shape = baseline;
                    translate_shape(shape, dx, dy);
                }
            }
            Some(Drag::Erase { radius }) => {
                let r = *radius;
                self.erase_at(p, r);
            }
            None => {}
        }
    }

    fn on_up(&mut self, p: FPoint) {
        let drag = match self.drag.take() {
            Some(d) => d,
            None => return,
        };
        match drag {
            Drag::Stroke { mut points } => {
                if points.len() < 2 {
                    return;
                }
                points.push(p);
                let style = current_style_for_canvas(self);
                let id = self.alloc_id();
                self.push_shape(Shape {
                    id,
                    kind: ShapeKind::FreehandStroke { points },
                    style,
                    rotation: 0.0,
                });
            }
            Drag::TwoPoint { from, to } => {
                let style = current_style_for_canvas(self);
                let kind = match &self.active_tool {
                    Tool::Line(_) => ShapeKind::Line { from, to },
                    Tool::Arrow(_) => ShapeKind::Arrow { from, to },
                    Tool::Rectangle(_) => ShapeKind::Rectangle {
                        rect: FRect::from_corners(from, to).to_int(),
                    },
                    Tool::Ellipse(_) => ShapeKind::Ellipse {
                        rect: FRect::from_corners(from, to).to_int(),
                    },
                    Tool::BlurRect { radius } => ShapeKind::BlurRect {
                        rect: FRect::from_corners(from, to).to_int(),
                        radius: *radius,
                    },
                    _ => return,
                };
                let id = self.alloc_id();
                self.push_shape(Shape {
                    id,
                    kind,
                    style,
                    rotation: 0.0,
                });
            }
            Drag::Region { from, to } => {
                self.region = Some(FRect::from_corners(from, to));
            }
            Drag::RegionMove { .. } | Drag::RegionResize { .. } => {
                // The region was already updated incrementally in `on_move`;
                // there's no commit step to perform.
            }
            Drag::Move { .. } | Drag::Erase { .. } => {
                // Already applied incrementally.
            }
        }
    }

    fn cancel_drag(&mut self) {
        self.drag = None;
    }

    // ----- text editing ----------------------------------------------------

    fn on_text_char(&mut self, c: char) {
        if let Some(pt) = self.pending_text.as_mut() {
            if !c.is_control() {
                pt.text.push(c);
            }
        }
    }

    fn on_text_backspace(&mut self) {
        if let Some(pt) = self.pending_text.as_mut() {
            pt.text.pop();
        }
    }

    fn commit_pending_text(&mut self) {
        if let Some(pt) = self.pending_text.take() {
            if pt.text.is_empty() {
                return;
            }
            let style = Style {
                stroke: pt.style.color,
                stroke_width: 1.0,
                fill: None,
            };
            let id = self.alloc_id();
            self.push_shape(Shape {
                id,
                kind: ShapeKind::Text {
                    origin: pt.origin,
                    content: pt.text,
                    style: pt.style,
                },
                style,
                rotation: 0.0,
            });
        }
    }

    // ----- step tool -------------------------------------------------------

    fn place_step(&mut self, center: FPoint, settings: StepSettings) {
        let number = self.next_step;
        self.next_step += 1;
        let style = Style::from(settings);
        let id = self.alloc_id();
        self.push_shape(Shape {
            id,
            kind: ShapeKind::Step {
                center,
                number,
                radius: settings.radius,
            },
            style,
            rotation: 0.0,
        });
    }

    // ----- eraser ----------------------------------------------------------

    fn erase_at(&mut self, p: FPoint, radius: f32) {
        let before = self.shapes.len();
        let pad = radius;
        self.shapes.retain(|s| {
            let b = s.bounds();
            let cx = b.x() as f32 + b.width() as f32 / 2.0;
            let cy = b.y() as f32 + b.height() as f32 / 2.0;
            let dist = ((cx - p.x).powi(2) + (cy - p.y).powi(2)).sqrt();
            // Keep if its bounding circle doesn't intersect the eraser.
            dist > pad + (b.width().max(b.height()) as f32) / 2.0 || !s.contains(p)
        });
        if self.shapes.len() != before {
            self.history.snapshot(&self.shapes);
        }
    }

    // ----- history ---------------------------------------------------------

    fn push_shape(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.history.snapshot(&self.shapes);
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.shapes.retain(|s| s.id != id);
            self.history.snapshot(&self.shapes);
        }
    }

    fn undo(&mut self) {
        if let Some(prev) = self.history.undo() {
            self.shapes = prev;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.history.redo() {
            self.shapes = next;
        }
    }

    fn alloc_id(&mut self) -> ShapeId {
        let id = ShapeId(self.next_id);
        self.next_id += 1;
        id
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Drag state machine
// ---------------------------------------------------------------------------

/// Resize handle on the region rectangle. Conceptually one of the 8 anchors
/// you'd see on a selection box: 4 corners + 4 edge midpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionHandle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

/// Pixel tolerance used to decide whether a click landed on a handle.
const HANDLE_TOL: f32 = 12.0;

/// Public wrapper used by the wayland layer driver to drive the cursor.
pub(crate) fn pointer_handle_pub(r: &Rect, p: FPoint) -> Option<RegionHandle> {
    pointer_handle(r, p)
}

fn region_contains(r: &Rect, p: FPoint) -> bool {
    p.x >= r.x() as f32
        && p.x <= (r.x() + r.width() as i32) as f32
        && p.y >= r.y() as f32
        && p.y <= (r.y() + r.height() as i32) as f32
}

/// Returns which handle (if any) the cursor `p` is grabbing on the region
/// rectangle. Edge handles win over moving when the click falls *exactly*
/// on the border.
fn pointer_handle(r: &Rect, p: FPoint) -> Option<RegionHandle> {
    let x0 = r.x() as f32;
    let y0 = r.y() as f32;
    let x1 = x0 + r.width() as f32;
    let y1 = y0 + r.height() as f32;
    let near_left = (p.x - x0).abs() <= HANDLE_TOL;
    let near_right = (p.x - x1).abs() <= HANDLE_TOL;
    let near_top = (p.y - y0).abs() <= HANDLE_TOL;
    let near_bottom = (p.y - y1).abs() <= HANDLE_TOL;
    let inside_x = p.x >= x0 - HANDLE_TOL && p.x <= x1 + HANDLE_TOL;
    let inside_y = p.y >= y0 - HANDLE_TOL && p.y <= y1 + HANDLE_TOL;
    if !inside_x || !inside_y {
        return None;
    }
    use RegionHandle::*;
    Some(match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => NW,
        (_, true, true, _) => NE,
        (true, _, _, true) => SW,
        (_, true, _, true) => SE,
        (_, _, true, _) => N,
        (_, _, _, true) => S,
        (true, _, _, _) => W,
        (_, true, _, _) => E,
        _ => return None,
    })
}

/// X offset (relative to the rectangle's `x()`) of the point the handle is
/// "anchored" to — used to compute the cursor delta during a resize drag.
fn handle_pivot_x(h: RegionHandle, r: &Rect) -> f32 {
    use RegionHandle::*;
    match h {
        NW | W | SW => 0.0,
        N | S => r.width() as f32 / 2.0,
        NE | E | SE => r.width() as f32,
    }
}

fn handle_pivot_y(h: RegionHandle, r: &Rect) -> f32 {
    use RegionHandle::*;
    match h {
        NW | N | NE => 0.0,
        W | E => r.height() as f32 / 2.0,
        SW | S | SE => r.height() as f32,
    }
}

/// Apply a resize offset (`dx`, `dy`) to the original region using the given
/// handle as the pivot.
fn resize_region(handle: RegionHandle, original: Rect, dx: i32, dy: i32) -> Rect {
    let mut x0 = original.x();
    let mut y0 = original.y();
    let mut x1 = original.x() + original.width() as i32;
    let mut y1 = original.y() + original.height() as i32;
    use RegionHandle::*;
    match handle {
        NW => {
            x0 += dx;
            y0 += dy;
        }
        N => y0 += dy,
        NE => {
            x1 += dx;
            y0 += dy;
        }
        E => x1 += dx,
        SE => {
            x1 += dx;
            y1 += dy;
        }
        S => y1 += dy,
        SW => {
            x0 += dx;
            y1 += dy;
        }
        W => x0 += dx,
    }
    if x1 < x0 {
        std::mem::swap(&mut x0, &mut x1);
    }
    if y1 < y0 {
        std::mem::swap(&mut y0, &mut y1);
    }
    Rect::from_xywh(x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)
}

#[derive(Clone, Debug)]
enum Drag {
    Stroke {
        points: Vec<FPoint>,
    },
    TwoPoint {
        from: FPoint,
        to: FPoint,
    },
    Region {
        from: FPoint,
        to: FPoint,
    },
    /// Translate the existing region rectangle by the cursor delta.
    RegionMove {
        start: FPoint,
        original: Rect,
    },
    /// Resize the existing region from one of its 8 handles.
    RegionResize {
        handle: RegionHandle,
        original: Rect,
    },
    Move {
        id: ShapeId,
        start: FPoint,
        /// Snapshot of the shape at drag-start. Every motion event
        /// reconstructs the shape's coordinates from this baseline + the
        /// (current - start) delta, so the move is *idempotent*: dragging
        /// in a circle ends up back at the original position. The
        /// previous implementation accumulated the delta on every motion,
        /// which made shapes fly off the screen.
        original_shape: Box<Shape>,
    },
    Erase {
        radius: f32,
    },
}

#[derive(Clone, Debug)]
struct PendingText {
    origin: FPoint,
    text: String,
    style: crate::shape::TextStyle,
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct History {
    undo: Vec<Vec<Shape>>,
    redo: Vec<Vec<Shape>>,
}

impl History {
    fn snapshot(&mut self, shapes: &[Shape]) {
        // Cap the stack so we don't grow forever during a long session.
        const MAX: usize = 100;
        self.undo.push(shapes.to_vec());
        if self.undo.len() > MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn undo(&mut self) -> Option<Vec<Shape>> {
        let cur = self.undo.pop()?;
        self.redo.push(cur.clone());
        self.undo.last().cloned()
    }
    fn redo(&mut self) -> Option<Vec<Shape>> {
        let next = self.redo.pop()?;
        self.undo.push(next.clone());
        Some(next)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn current_style(tool: &Tool) -> Style {
    match tool {
        Tool::Brush(b)
        | Tool::Line(b)
        | Tool::Arrow(b)
        | Tool::Rectangle(b)
        | Tool::Ellipse(b)
        | Tool::Polygon(b) => Style::from(*b),
        Tool::BlurRect { .. } => Style {
            stroke: crate::color::Color::ACCENT,
            stroke_width: 1.0,
            fill: Some(crate::color::Color::SHADOW),
        },
        Tool::Step(s) => Style::from(*s),
        Tool::Text(_) | Tool::Pointer | Tool::Eraser { .. } => Style::default(),
    }
}

/// Style for the current tool with `fill_mode` honoured — closed shapes
/// (rect / ellipse) get their interior filled with the stroke colour
/// when the toolbar's FILL toggle is on.
fn current_style_with_fill(tool: &Tool, fill_mode: bool) -> Style {
    let mut s = current_style(tool);
    if fill_mode
        && matches!(
            tool,
            Tool::Rectangle(_) | Tool::Ellipse(_) | Tool::Polygon(_)
        )
    {
        s.fill = Some(s.stroke);
    }
    s
}

/// Apply the "constrain" modifier (Shift) to a two-point drag's endpoint
/// based on the active tool:
///   * Lines / arrows → snap the angle to the nearest 45°.
///   * Rectangles / ellipses / blur rectangles → force a square (the
///     larger of the two deltas wins, preserving the cursor's quadrant).
fn apply_constraint(tool: &Tool, from: FPoint, to: FPoint) -> FPoint {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    match tool {
        Tool::Line(_) | Tool::Arrow(_) => {
            let len = (dx * dx + dy * dy).sqrt();
            if len < f32::EPSILON {
                return from;
            }
            let angle = dy.atan2(dx);
            let step = std::f32::consts::FRAC_PI_4; // 45°
            let snapped = (angle / step).round() * step;
            FPoint::new(from.x + len * snapped.cos(), from.y + len * snapped.sin())
        }
        Tool::Rectangle(_) | Tool::Ellipse(_) | Tool::BlurRect { .. } => {
            let side = dx.abs().max(dy.abs());
            let sx = if dx >= 0.0 { 1.0 } else { -1.0 };
            let sy = if dy >= 0.0 { 1.0 } else { -1.0 };
            FPoint::new(from.x + side * sx, from.y + side * sy)
        }
        _ => to,
    }
}

/// Variant of [`current_style_with_fill`] that uses an explicit fill colour
/// (when the user has set one via Shift+click) instead of the stroke colour.
fn current_style_for_canvas(canvas: &Canvas) -> Style {
    let mut s = current_style_with_fill(&canvas.active_tool, canvas.fill_mode);
    if canvas.fill_mode {
        if let Some(c) = canvas.fill_color {
            s.fill = Some(c);
        }
    }
    s
}

/// Public wrapper for [`scale_shape`] that takes an `FPoint` anchor.
pub fn scale_shape_about(shape: &mut Shape, anchor: FPoint, factor: f32) {
    scale_shape(shape, anchor.x, anchor.y, factor);
}

/// Public wrapper for [`rotate_shape`] that takes an `FPoint` centre.
pub fn rotate_shape_about(shape: &mut Shape, center: FPoint, radians: f32) {
    rotate_shape(shape, center.x, center.y, radians);
}

/// Scale every coordinate of `shape` away from `(cx, cy)` by `factor`.
fn scale_shape(shape: &mut Shape, cx: f32, cy: f32, factor: f32) {
    let s = |p: &mut FPoint| {
        p.x = cx + (p.x - cx) * factor;
        p.y = cy + (p.y - cy) * factor;
    };
    match &mut shape.kind {
        ShapeKind::FreehandStroke { points } | ShapeKind::Polygon { points, .. } => {
            for p in points.iter_mut() {
                s(p);
            }
        }
        ShapeKind::Line { from, to } | ShapeKind::Arrow { from, to } => {
            s(from);
            s(to);
        }
        ShapeKind::Rectangle { rect }
        | ShapeKind::Ellipse { rect }
        | ShapeKind::BlurRect { rect, .. } => {
            let mut tl = FPoint::new(rect.x() as f32, rect.y() as f32);
            let mut br = FPoint::new(
                rect.x() as f32 + rect.width() as f32,
                rect.y() as f32 + rect.height() as f32,
            );
            s(&mut tl);
            s(&mut br);
            let nx = tl.x.min(br.x) as i32;
            let ny = tl.y.min(br.y) as i32;
            let nw = (br.x - tl.x).abs().max(1.0) as u32;
            let nh = (br.y - tl.y).abs().max(1.0) as u32;
            *rect = sss_capture::Rect::from_xywh(nx, ny, nw, nh);
        }
        ShapeKind::Step { center, radius, .. } => {
            s(center);
            *radius = (*radius * factor).max(2.0);
        }
        ShapeKind::Text { origin, style, .. } => {
            s(origin);
            style.size = (style.size * factor).max(6.0);
        }
    }
    shape.style.stroke_width = (shape.style.stroke_width * factor).max(0.5);
}

/// Rotate every coordinate of `shape` by `radians` about `(cx, cy)`.
fn rotate_shape(shape: &mut Shape, cx: f32, cy: f32, radians: f32) {
    let (sn, cs) = radians.sin_cos();
    let r = |p: &mut FPoint| {
        let dx = p.x - cx;
        let dy = p.y - cy;
        p.x = cx + dx * cs - dy * sn;
        p.y = cy + dx * sn + dy * cs;
    };
    match &mut shape.kind {
        ShapeKind::FreehandStroke { points } | ShapeKind::Polygon { points, .. } => {
            for p in points.iter_mut() {
                r(p);
            }
        }
        ShapeKind::Line { from, to } | ShapeKind::Arrow { from, to } => {
            r(from);
            r(to);
        }
        ShapeKind::Rectangle { rect } => {
            // Axis-aligned `Rectangle` can't store a free angle, so the
            // rotation converts it into a 4-point `Polygon` whose
            // vertices live at the rotated corners. Visually identical
            // for 0°/90°/180°/270°, but unlike the previous AABB
            // re-bound it actually *looks* rotated for intermediate
            // angles. Fill carries over via `shape.style`.
            let pts = rect_corners(*rect);
            let rotated: Vec<FPoint> = pts
                .into_iter()
                .map(|mut p| {
                    r(&mut p);
                    p
                })
                .collect();
            shape.kind = ShapeKind::Polygon {
                points: rotated,
                closed: true,
            };
        }
        ShapeKind::Ellipse { rect } => {
            // Same trick: approximate the ellipse with a 64-point
            // polygon and rotate that. The rasteriser handles polygons
            // with curved-looking outlines just fine.
            let cx0 = rect.x() as f32 + rect.width() as f32 / 2.0;
            let cy0 = rect.y() as f32 + rect.height() as f32 / 2.0;
            let rx = rect.width() as f32 / 2.0;
            let ry = rect.height() as f32 / 2.0;
            let n = 64;
            let mut pts = Vec::with_capacity(n);
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                let mut p = FPoint::new(cx0 + a.cos() * rx, cy0 + a.sin() * ry);
                r(&mut p);
                pts.push(p);
            }
            shape.kind = ShapeKind::Polygon {
                points: pts,
                closed: true,
            };
        }
        ShapeKind::BlurRect { rect, .. } => {
            // BlurRect is special — its rasteriser needs an axis-aligned
            // rect. Keep the AABB re-bound for now; rotating blur
            // requires per-pixel sampling which we don't support.
            let pts = rect_corners(*rect);
            let mut rotated = pts;
            for p in rotated.iter_mut() {
                r(p);
            }
            let xs: Vec<f32> = rotated.iter().map(|p| p.x).collect();
            let ys: Vec<f32> = rotated.iter().map(|p| p.y).collect();
            let nx = xs.iter().cloned().fold(f32::INFINITY, f32::min) as i32;
            let ny = ys.iter().cloned().fold(f32::INFINITY, f32::min) as i32;
            let mx = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as i32;
            let my = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as i32;
            *rect = sss_capture::Rect::from_xywh(
                nx,
                ny,
                (mx - nx).max(1) as u32,
                (my - ny).max(1) as u32,
            );
        }
        ShapeKind::Step { center, .. } => r(center),
        ShapeKind::Text { origin, .. } => r(origin),
    }
}

fn rect_corners(rect: sss_capture::Rect) -> [FPoint; 4] {
    let x0 = rect.x() as f32;
    let y0 = rect.y() as f32;
    let x1 = x0 + rect.width() as f32;
    let y1 = y0 + rect.height() as f32;
    [
        FPoint::new(x0, y0),
        FPoint::new(x1, y0),
        FPoint::new(x1, y1),
        FPoint::new(x0, y1),
    ]
}

/// Move every coordinate of `shape` by `(dx, dy)` relative to its original
/// (pre-drag) bounding box.
fn translate_shape(shape: &mut Shape, dx: i32, dy: i32) {
    let dx_f = dx as f32;
    let dy_f = dy as f32;
    match &mut shape.kind {
        ShapeKind::FreehandStroke { points } | ShapeKind::Polygon { points, .. } => {
            for p in points.iter_mut() {
                p.x += dx_f;
                p.y += dy_f;
            }
        }
        ShapeKind::Line { from, to } | ShapeKind::Arrow { from, to } => {
            from.x += dx_f;
            from.y += dy_f;
            to.x += dx_f;
            to.y += dy_f;
        }
        ShapeKind::Rectangle { rect }
        | ShapeKind::Ellipse { rect }
        | ShapeKind::BlurRect { rect, .. } => {
            *rect = sss_capture::Rect::from_xywh(
                rect.x() + dx,
                rect.y() + dy,
                rect.width(),
                rect.height(),
            );
        }
        ShapeKind::Step { center, .. } => {
            center.x += dx_f;
            center.y += dy_f;
        }
        ShapeKind::Text { origin, .. } => {
            origin.x += dx_f;
            origin.y += dy_f;
        }
    }
}
