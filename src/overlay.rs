//! Pane-local window ownership, stacking, focus and gesture state for terminal overlays.
//!
//! Geometry and hit-test compilation are supplied by the presenter. This state machine never
//! performs OS input injection and never treats a surface ID without its owner as an identity.

use crate::identity::{SessionIdentity, SurfaceIdentity};
use crate::vector::{HitRole, InvalidScene, Point, Rect, Scalar};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub mod wire;

pub const MAX_WINDOWS: usize = 256;
pub const MAX_WINDOWS_PER_OWNER: usize = 32;
pub const MAX_PENDING_EVENTS: usize = 256;
pub const MAX_EVENT_TEXT_BYTES: usize = 4096;
pub const MAX_EVENT_QUEUE_BYTES: usize = 128 * 1024;
pub const MAX_OWNERS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Floating,
    Popup,
    Modal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOptions {
    pub bounds: Rect,
    pub mode: WindowMode,
    pub visible: bool,
    pub parent: Option<SurfaceIdentity>,
    pub min_width: Scalar,
    pub min_height: Scalar,
}
impl WindowOptions {
    pub fn new(bounds: Rect, mode: WindowMode) -> Self {
        Self {
            bounds,
            mode,
            visible: true,
            parent: None,
            min_width: Scalar::ONE,
            min_height: Scalar::ONE,
        }
    }
    pub fn validate(&self) -> Result<(), InvalidScene> {
        self.bounds.validate()?;
        if self.min_width <= Scalar::ZERO
            || self.min_height <= Scalar::ZERO
            || self.bounds.width < self.min_width
            || self.bounds.height < self.min_height
        {
            return Err(InvalidScene("window bounds are smaller than minimum size"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissReason {
    Escape,
    OutsidePress,
    Closed,
    OwnerLost,
    ParentClosed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Focus(bool),
    Pointer {
        position: Point,
        region: u64,
        button: Option<(u16, bool)>,
        modifiers: u32,
    },
    Wheel {
        position: Point,
        dx: Scalar,
        dy: Scalar,
        modifiers: u32,
    },
    Key {
        physical: u32,
        down: bool,
        repeat: bool,
        modifiers: u32,
    },
    Text(String),
    Ime {
        preedit: String,
        selection: Option<(u32, u32)>,
    },
    Geometry {
        bounds: Rect,
        settled: bool,
    },
    Dismissed(DismissReason),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEvent {
    pub window: SurfaceIdentity,
    pub generation: u64,
    pub revision: u64,
    /// Published drawing/hit-test revision, independent of the geometry revision.
    pub scene_revision: u64,
    pub event: Event,
}

#[derive(Debug, Clone)]
pub struct Window {
    pub identity: SurfaceIdentity,
    pub generation: u64,
    pub revision: u64,
    pub scene_revision: u64,
    pub options: WindowOptions,
}

#[derive(Debug, Clone, Copy)]
struct Capture {
    window: SurfaceIdentity,
    role: HitRole,
    start: Point,
    bounds: Rect,
    region: u64,
}

#[derive(Debug, Default)]
pub struct Windows {
    windows: BTreeMap<SurfaceIdentity, Window>,
    stack: Vec<SurfaceIdentity>,
    focus: Option<SurfaceIdentity>,
    focus_history: Vec<SurfaceIdentity>,
    swallowed_buttons: BTreeSet<u16>,
    swallowed_escape: bool,
    capture: Option<Capture>,
    events: BTreeMap<SessionIdentity, VecDeque<WindowEvent>>,
}

impl Windows {
    pub fn get(&self, id: SurfaceIdentity) -> Option<&Window> {
        self.windows.get(&id)
    }
    pub fn focus(&self) -> Option<SurfaceIdentity> {
        self.focus
    }
    pub fn visible(&self) -> impl Iterator<Item = &Window> {
        self.stack
            .iter()
            .filter_map(|id| self.windows.get(id))
            .filter(|w| self.effectively_visible(w.identity))
    }
    pub fn create(
        &mut self,
        id: SurfaceIdentity,
        generation: u64,
        options: WindowOptions,
    ) -> Result<(), InvalidScene> {
        options.validate()?;
        if generation == 0 || id.context.context_id == 0 || id.surface_id == 0 {
            return Err(InvalidScene(
                "window identity and generation must be nonzero",
            ));
        }
        if self.windows.contains_key(&id) {
            return Err(InvalidScene("window already exists"));
        }
        if !self.events.contains_key(&id.context.session) && self.events.len() >= MAX_OWNERS {
            return Err(InvalidScene("overlay owner limit exceeded"));
        }
        if self.windows.len() >= MAX_WINDOWS
            || self
                .windows
                .keys()
                .filter(|key| key.context.session == id.context.session)
                .count()
                >= MAX_WINDOWS_PER_OWNER
        {
            return Err(InvalidScene("window limit exceeded"));
        }
        if let Some(parent) = options.parent {
            if parent.context.session != id.context.session || !self.windows.contains_key(&parent) {
                return Err(InvalidScene("invalid parent owner or lifecycle"));
            }
        }
        if options.visible
            && options.mode == WindowMode::Modal
            && self
                .top_modal()
                .is_some_and(|m| m.context.session != id.context.session)
        {
            return Err(InvalidScene("another owner holds the modal focus"));
        }
        let take_focus = options.visible && options.mode != WindowMode::Floating;
        if take_focus
            && options.mode == WindowMode::Popup
            && self
                .top_modal()
                .is_some_and(|modal| !options.parent.is_some_and(|p| self.descendant(p, modal)))
        {
            return Err(InvalidScene("popup must descend from the active modal"));
        }
        self.windows.insert(
            id,
            Window {
                identity: id,
                generation,
                revision: 1,
                scene_revision: 0,
                options,
            },
        );
        self.events.entry(id.context.session).or_default();
        self.stack.push(id);
        if take_focus {
            self.request_focus(id)?;
        }
        Ok(())
    }
    /// Commit the revision only when the presenter has atomically installed drawing and hits.
    pub fn publish_scene(
        &mut self,
        id: SurfaceIdentity,
        generation: u64,
        scene_revision: u64,
    ) -> Result<(), InvalidScene> {
        let window = self
            .windows
            .get_mut(&id)
            .ok_or(InvalidScene("window does not exist"))?;
        if window.generation != generation
            || scene_revision == 0
            || scene_revision <= window.scene_revision
        {
            return Err(InvalidScene("stale scene generation or revision"));
        }
        window.scene_revision = scene_revision;
        Ok(())
    }
    /// Apply a conditional window action. Close returns no surviving window revision.
    pub fn apply_action(
        &mut self,
        owner: SessionIdentity,
        action: wire::Action,
        viewport: wire::Viewport,
    ) -> Result<Option<u64>, InvalidScene> {
        action
            .payload()
            .map_err(|_| InvalidScene("invalid window action"))?;
        let id = action
            .address
            .identity(owner)
            .map_err(|_| InvalidScene("invalid window identity"))?;
        let old = self
            .windows
            .get(&id)
            .ok_or(InvalidScene("window does not exist"))?;
        if old.generation != action.address.generation || old.revision != action.expected_revision {
            return Err(InvalidScene("stale window generation or revision"));
        }
        if action.action == wire::WindowAction::Close {
            self.close(id, DismissReason::Closed);
            return Ok(None);
        }
        let next = old
            .revision
            .checked_add(1)
            .ok_or(InvalidScene("window revision exhausted"))?;
        let centered = if action.action == wire::WindowAction::Center {
            viewport
                .validate()
                .map_err(|_| InvalidScene("invalid viewport"))?;
            Some(Rect::new(
                ((viewport.width.get() - old.options.bounds.width.get()) / 2.).max(0.),
                ((viewport.height.get() - old.options.bounds.height.get()) / 2.).max(0.),
                old.options.bounds.width.get(),
                old.options.bounds.height.get(),
            )?)
        } else {
            None
        };
        match action.action {
            wire::WindowAction::Focus => self.request_focus(id)?,
            wire::WindowAction::Raise => {
                if !self.eligible(id) {
                    return Err(InvalidScene("window is hidden or blocked by a modal"));
                }
                self.raise_group(id);
            }
            wire::WindowAction::Lower => {
                if !self.eligible(id) || self.top_modal().is_some() {
                    return Err(InvalidScene(
                        "lowering is blocked by visibility or an active modal",
                    ));
                }
                let group: Vec<_> = self
                    .stack
                    .iter()
                    .copied()
                    .filter(|child| self.descendant(*child, id))
                    .collect();
                self.stack.retain(|key| !group.contains(key));
                let index = self
                    .windows
                    .get(&id)
                    .and_then(|w| w.options.parent)
                    .and_then(|p| self.stack.iter().position(|key| *key == p))
                    .map_or(0, |index| index + 1);
                self.stack.splice(index..index, group);
            }
            wire::WindowAction::Center => {}
            wire::WindowAction::Close => unreachable!("close handled before advancing revision"),
        }
        let window = self
            .windows
            .get_mut(&id)
            .ok_or(InvalidScene("window lost during action"))?;
        window.revision = next;
        if let Some(bounds) = centered {
            window.options.bounds = bounds;
            self.emit(
                id,
                Event::Geometry {
                    bounds,
                    settled: true,
                },
            );
        }
        Ok(Some(next))
    }

    fn raise_group(&mut self, id: SurfaceIdentity) {
        // A parent cannot paint over its own popups when it is raised or focused.
        let group: Vec<_> = self
            .stack
            .iter()
            .copied()
            .filter(|child| self.descendant(*child, id))
            .collect();
        self.stack.retain(|key| !group.contains(key));
        self.stack.extend(group);
    }
    pub fn update(
        &mut self,
        id: SurfaceIdentity,
        generation: u64,
        revision: u64,
        options: WindowOptions,
    ) -> Result<u64, InvalidScene> {
        options.validate()?;
        let old = self
            .windows
            .get(&id)
            .ok_or(InvalidScene("window does not exist"))?;
        if old.generation != generation || old.revision != revision {
            return Err(InvalidScene("stale window generation or revision"));
        }
        if options.parent != old.options.parent || options.mode != old.options.mode {
            return Err(InvalidScene("window parent and mode are immutable"));
        }
        if options.visible
            && options.mode == WindowMode::Modal
            && self
                .top_modal()
                .is_some_and(|m| m.context.session != id.context.session)
        {
            return Err(InvalidScene("another owner holds modal focus"));
        }
        if options.visible
            && options.mode == WindowMode::Popup
            && self
                .top_modal()
                .is_some_and(|modal| !self.descendant(id, modal))
        {
            return Err(InvalidScene("popup is blocked by the active modal"));
        }
        let next = revision
            .checked_add(1)
            .ok_or(InvalidScene("window revision exhausted"))?;
        let hiding = old.options.visible && !options.visible;
        let showing =
            !old.options.visible && options.visible && options.mode != WindowMode::Floating;
        let w = self
            .windows
            .get_mut(&id)
            .ok_or(InvalidScene("window does not exist"))?;
        w.options = options;
        w.revision = next;
        if hiding {
            let descendants: Vec<_> = self
                .stack
                .iter()
                .copied()
                .filter(|child| self.descendant(*child, id))
                .collect();
            for child in descendants {
                self.cancel_capture(child);
            }
            if self
                .focus
                .is_some_and(|focused| self.descendant(focused, id))
            {
                self.change_focus(None);
                self.restore_focus();
            }
        }
        if showing {
            self.request_focus(id)?;
        }
        Ok(next)
    }
    fn top_modal(&self) -> Option<SurfaceIdentity> {
        self.visible()
            .filter(|w| w.options.mode == WindowMode::Modal)
            .last()
            .map(|w| w.identity)
    }
    fn descendant(&self, mut id: SurfaceIdentity, parent: SurfaceIdentity) -> bool {
        for _ in 0..MAX_WINDOWS {
            if id == parent {
                return true;
            }
            let Some(next) = self.windows.get(&id).and_then(|w| w.options.parent) else {
                return false;
            };
            id = next;
        }
        false
    }
    fn effectively_visible(&self, mut id: SurfaceIdentity) -> bool {
        for _ in 0..MAX_WINDOWS {
            let Some(w) = self.windows.get(&id) else {
                return false;
            };
            if !w.options.visible {
                return false;
            }
            match w.options.parent {
                Some(parent) => id = parent,
                None => return true,
            }
        }
        false
    }
    fn eligible(&self, id: SurfaceIdentity) -> bool {
        self.effectively_visible(id)
            && self
                .top_modal()
                .is_none_or(|modal| self.descendant(id, modal))
    }
    pub fn request_focus(&mut self, id: SurfaceIdentity) -> Result<(), InvalidScene> {
        if !self.eligible(id) {
            return Err(InvalidScene("window is hidden or blocked by a modal"));
        }
        self.raise_group(id);
        self.focus_history.retain(|key| *key != id);
        self.focus_history.push(id);
        self.change_focus(Some(id));
        Ok(())
    }
    fn change_focus(&mut self, next: Option<SurfaceIdentity>) {
        if self.focus == next {
            return;
        }
        if let Some(old) = self.focus.take() {
            self.emit(old, Event::Cancel);
            self.emit(old, Event::Focus(false));
        }
        self.focus = next;
        if let Some(id) = next {
            self.emit(id, Event::Focus(true));
        }
    }
    fn restore_focus(&mut self) {
        let next = self
            .focus_history
            .iter()
            .rev()
            .copied()
            .find(|id| self.eligible(*id));
        self.change_focus(next);
    }
    pub fn close(&mut self, id: SurfaceIdentity, reason: DismissReason) {
        // Remove descendants first, without allowing a parent of a removed node to keep input.
        let children: Vec<_> = self
            .stack
            .iter()
            .rev()
            .copied()
            .filter(|child| *child != id && self.descendant(*child, id))
            .collect();
        for child in children {
            self.remove_one(child, DismissReason::ParentClosed);
        }
        self.remove_one(id, reason);
        self.restore_focus();
    }
    fn remove_one(&mut self, id: SurfaceIdentity, reason: DismissReason) {
        self.cancel_capture(id);
        if self.focus == Some(id) {
            self.change_focus(None);
        }
        self.emit(id, Event::Dismissed(reason));
        self.windows.remove(&id);
        self.stack.retain(|key| *key != id);
        self.focus_history.retain(|key| *key != id);
    }
    pub fn revoke_owner(&mut self, owner: SessionIdentity) {
        let ids: Vec<_> = self
            .stack
            .iter()
            .copied()
            .filter(|id| id.context.session == owner)
            .collect();
        // Do not enqueue more events to an owner that cannot receive them.
        self.events.remove(&owner);
        self.focus_history.retain(|id| id.context.session != owner);
        for id in ids {
            self.windows.remove(&id);
            self.stack.retain(|key| *key != id);
        }
        if self
            .capture
            .is_some_and(|c| c.window.context.session == owner)
        {
            self.capture = None;
        }
        if self.focus.is_some_and(|id| id.context.session == owner) {
            self.focus = None;
            self.restore_focus();
        }
    }
    fn emit(&mut self, id: SurfaceIdentity, event: Event) {
        if !valid_event(&event) {
            self.revoke_owner(id.context.session);
            return;
        }
        let Some(w) = self.windows.get(&id) else {
            return;
        };
        let e = WindowEvent {
            window: id,
            generation: w.generation,
            revision: w.revision,
            scene_revision: w.scene_revision,
            event,
        };
        let Some(queue) = self.events.get_mut(&id.context.session) else {
            return;
        };
        // Only replace adjacent motion/geometry updates; never drop key/button transitions.
        let coalesces = queue.back().is_some_and(|last| {
            last.window == id
                && last.scene_revision == e.scene_revision
                && matches!(
                    (&last.event, &e.event),
                    (
                        Event::Pointer { button: None, .. },
                        Event::Pointer { button: None, .. }
                    ) | (
                        Event::Geometry { settled: false, .. },
                        Event::Geometry { settled: false, .. }
                    )
                )
        });
        if coalesces {
            queue.pop_back();
        }
        if queue.len() >= MAX_PENDING_EVENTS
            || queue
                .iter()
                .map(|event| event_bytes(&event.event))
                .sum::<usize>()
                + event_bytes(&e.event)
                > MAX_EVENT_QUEUE_BYTES
        {
            self.revoke_owner(id.context.session);
            return;
        }
        queue.push_back(e);
    }
    pub fn take_event(&mut self, owner: SessionIdentity) -> Option<WindowEvent> {
        let queue = self.events.get_mut(&owner)?;
        let event = queue.pop_front();
        if queue.is_empty() && !self.windows.keys().any(|key| key.context.session == owner) {
            self.events.remove(&owner);
        }
        event
    }
    pub fn capture_pointer(
        &mut self,
        id: SurfaceIdentity,
        position: Point,
    ) -> Result<(), InvalidScene> {
        if !self.eligible(id) || self.focus != Some(id) {
            return Err(InvalidScene("pointer capture requires eligible focus"));
        }
        let w = self
            .windows
            .get(&id)
            .ok_or(InvalidScene("window does not exist"))?;
        self.capture = Some(Capture {
            window: id,
            role: HitRole::Input,
            start: position,
            bounds: w.options.bounds,
            region: 0,
        });
        Ok(())
    }
    pub fn release_pointer(&mut self, id: SurfaceIdentity) {
        if self.capture.is_some_and(|c| c.window == id) {
            self.capture = None;
        }
    }
    fn cancel_capture(&mut self, id: SurfaceIdentity) {
        if self.capture.is_some_and(|c| c.window == id) {
            self.capture = None;
            self.emit(id, Event::Cancel);
        }
    }
    /// Route ordinary keys only after the host has handled its reserved application shortcuts.
    pub fn keyboard(&mut self, event: Event, escape: bool) -> bool {
        if escape && self.swallowed_escape {
            if matches!(event, Event::Key { down: false, .. }) {
                self.swallowed_escape = false;
            }
            return true;
        }
        let Some(id) = self.focus.filter(|id| self.eligible(*id)) else {
            return self.top_modal().is_some();
        };
        if escape
            && matches!(
                event,
                Event::Key {
                    down: true,
                    repeat: false,
                    ..
                }
            )
            && self
                .windows
                .get(&id)
                .is_some_and(|w| w.options.mode != WindowMode::Floating)
        {
            self.swallowed_escape = true;
            self.close(id, DismissReason::Escape);
            return true;
        }
        self.emit(id, event);
        true
    }
    /// Wheel input follows hit geometry and modal eligibility without changing keyboard focus.
    pub fn wheel(
        &mut self,
        position: Point,
        dx: Scalar,
        dy: Scalar,
        modifiers: u32,
        hit: impl Fn(SurfaceIdentity, Point) -> Option<(u64, HitRole)>,
    ) -> bool {
        let target = self
            .stack
            .iter()
            .rev()
            .copied()
            .filter(|id| self.eligible(*id))
            .find_map(|id| {
                let window = self.windows.get(&id)?;
                if !window.options.bounds.contains(position) {
                    return None;
                }
                let local = self.local(id, position)?;
                let (_, role) = hit(id, local)?;
                (role != HitRole::Transparent).then_some((id, local))
            });
        if let Some((id, position)) = target {
            self.emit(
                id,
                Event::Wheel {
                    position,
                    dx,
                    dy,
                    modifiers,
                },
            );
            true
        } else {
            self.top_modal().is_some()
        }
    }
    /// Hit-test callback uses the same compiled paths, transforms and clips as the draw scene.
    /// None is input-transparent; a default rectangular hit is the compiler's responsibility.
    pub fn pointer(
        &mut self,
        position: Point,
        button: Option<(u16, bool)>,
        modifiers: u32,
        hit: impl Fn(SurfaceIdentity, Point) -> Option<(u64, HitRole)>,
    ) -> bool {
        if let Some((button_id, down)) = button {
            if self.swallowed_buttons.contains(&button_id) {
                if !down {
                    self.swallowed_buttons.remove(&button_id);
                }
                return true;
            }
        }
        if let Some(capture) = self.capture {
            if !self.eligible(capture.window) {
                self.cancel_capture(capture.window);
                return self.top_modal().is_some();
            }
            if matches!(capture.role, HitRole::Drag | HitRole::Resize(_)) {
                self.gesture(capture, position, button.is_some_and(|(_, down)| !down));
            } else if let Some(local) = self.local(capture.window, position) {
                self.emit(
                    capture.window,
                    Event::Pointer {
                        position: local,
                        region: capture.region,
                        button,
                        modifiers,
                    },
                );
            }
            if button.is_some_and(|(_, down)| !down) {
                self.capture = None;
            }
            return true;
        }
        // An outside press dismisses the foremost popup and is consumed, including its release.
        let popup = self
            .visible()
            .filter(|w| w.options.mode == WindowMode::Popup && self.eligible(w.identity))
            .last()
            .map(|w| (w.identity, w.options.bounds));
        if let Some((id, bounds)) = popup {
            if button.is_some_and(|(_, down)| down) && !bounds.contains(position) {
                if let Some((button_id, _)) = button {
                    self.swallowed_buttons.insert(button_id);
                }
                self.close(id, DismissReason::OutsidePress);
                return true;
            }
        }
        let target = self
            .stack
            .iter()
            .rev()
            .copied()
            .filter(|id| self.eligible(*id))
            .find_map(|id| {
                let w = self.windows.get(&id)?;
                if !w.options.bounds.contains(position) {
                    return None;
                }
                let local = self.local(id, position)?;
                let (region, role) = hit(id, local)?;
                if role == HitRole::Transparent {
                    return None;
                }
                Some((id, local, region, role))
            });
        let Some((id, local, region, role)) = target else {
            let blocked = self.top_modal().is_some();
            if !blocked && button.is_some_and(|(_, down)| down) {
                self.change_focus(None);
                self.focus_history.clear();
            }
            return blocked;
        };
        if button.is_some_and(|(_, down)| down) {
            let _ = self.request_focus(id);
            if matches!(role, HitRole::Drag | HitRole::Resize(_))
                && let Some(w) = self.windows.get(&id)
            {
                self.capture = Some(Capture {
                    window: id,
                    role,
                    start: position,
                    bounds: w.options.bounds,
                    region,
                });
            }
        }
        self.emit(
            id,
            Event::Pointer {
                position: local,
                region,
                button,
                modifiers,
            },
        );
        true
    }
    fn local(&self, id: SurfaceIdentity, p: Point) -> Option<Point> {
        let r = self.windows.get(&id)?.options.bounds;
        Point::new(p.x.get() - r.origin.x.get(), p.y.get() - r.origin.y.get()).ok()
    }
    fn gesture(&mut self, c: Capture, p: Point, settled: bool) {
        let Some(w) = self.windows.get(&c.window) else {
            return;
        };
        let dx = p.x.get() - c.start.x.get();
        let dy = p.y.get() - c.start.y.get();
        let (mut x, mut y, mut width, mut height) = (
            c.bounds.origin.x.get(),
            c.bounds.origin.y.get(),
            c.bounds.width.get(),
            c.bounds.height.get(),
        );
        match c.role {
            HitRole::Drag => {
                x += dx;
                y += dy;
            }
            HitRole::Resize(edges) => {
                if edges & 1 != 0 {
                    width = (width - dx).max(w.options.min_width.get());
                    x += c.bounds.width.get() - width;
                }
                if edges & 2 != 0 {
                    height = (height - dy).max(w.options.min_height.get());
                    y += c.bounds.height.get() - height;
                }
                if edges & 4 != 0 {
                    width = (width + dx).max(w.options.min_width.get());
                }
                if edges & 8 != 0 {
                    height = (height + dy).max(w.options.min_height.get());
                }
            }
            _ => return,
        }
        let Ok(bounds) = Rect::new(x, y, width, height) else {
            return;
        };
        let Some(revision) = w.revision.checked_add(1) else {
            self.close(c.window, DismissReason::Closed);
            return;
        };
        if let Some(w) = self.windows.get_mut(&c.window) {
            w.options.bounds = bounds;
            w.revision = revision;
        }
        self.emit(c.window, Event::Geometry { bounds, settled });
    }
}

fn event_bytes(event: &Event) -> usize {
    128 + match event {
        Event::Text(text) => text.len(),
        Event::Ime { preedit, .. } => preedit.len(),
        _ => 0,
    }
}
fn valid_event(event: &Event) -> bool {
    match event {
        Event::Geometry { bounds, .. } => bounds.validate().is_ok(),
        Event::Text(text) => text.len() <= MAX_EVENT_TEXT_BYTES,
        Event::Ime { preedit, selection } => {
            preedit.len() <= MAX_EVENT_TEXT_BYTES
                && selection.is_none_or(|(start, end)| {
                    start <= end
                        && (end as usize) <= preedit.len()
                        && preedit.is_char_boundary(start as usize)
                        && preedit.is_char_boundary(end as usize)
                })
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PresenterInstanceId;
    fn key(owner: u64, id: u64) -> SurfaceIdentity {
        SessionIdentity::new(PresenterInstanceId([1; 16]), owner)
            .unwrap()
            .context(1)
            .unwrap()
            .surface(id)
            .unwrap()
    }
    fn options(mode: WindowMode) -> WindowOptions {
        WindowOptions::new(Rect::new(10.0, 10.0, 100.0, 80.0).unwrap(), mode)
    }
    #[test]
    fn window_actions_check_owner_revision_and_keep_popups_above_their_parent() {
        let a = key(1, 1);
        let popup = key(1, 2);
        let b = key(2, 1);
        let mut state = Windows::default();
        state.create(a, 1, options(WindowMode::Floating)).unwrap();
        state.create(b, 1, options(WindowMode::Floating)).unwrap();
        let mut child = options(WindowMode::Popup);
        child.parent = Some(a);
        state.create(popup, 1, child).unwrap();
        let viewport = wire::Viewport {
            width: Scalar::new(800.).unwrap(),
            height: Scalar::new(600.).unwrap(),
            scale_numerator: 3,
            scale_denominator: 2,
        };
        let action = wire::Action {
            address: wire::WindowAddress {
                context_id: 1,
                surface_id: 1,
                generation: 1,
            },
            expected_revision: 1,
            action: wire::WindowAction::Raise,
        };
        assert_eq!(
            state
                .apply_action(a.context.session, action, viewport)
                .unwrap(),
            Some(2)
        );
        let order: Vec<_> = state.visible().map(|w| w.identity).collect();
        assert_eq!(order, [b, a, popup]);
        assert_eq!(state.get(b).unwrap().revision, 1);
        assert!(
            state
                .apply_action(a.context.session, action, viewport)
                .is_err()
        );
        assert_eq!(
            state.visible().map(|w| w.identity).collect::<Vec<_>>(),
            order
        );
        state
            .apply_action(
                a.context.session,
                wire::Action {
                    expected_revision: 2,
                    action: wire::WindowAction::Center,
                    ..action
                },
                viewport,
            )
            .unwrap();
        assert_eq!(
            state.get(a).unwrap().options.bounds.origin,
            Point::new(350., 260.).unwrap()
        );
        state
            .apply_action(
                a.context.session,
                wire::Action {
                    expected_revision: 3,
                    action: wire::WindowAction::Close,
                    ..action
                },
                viewport,
            )
            .unwrap();
        assert!(state.get(a).is_none());
        assert!(state.get(popup).is_none());
        assert!(state.get(b).is_some());
    }
    #[test]
    fn published_scene_revision_survives_geometry_changes_and_prevents_cross_revision_coalescing() {
        let a = key(1, 1);
        let b = key(2, 1);
        let mut state = Windows::default();
        for id in [a, b] {
            state.create(id, 1, options(WindowMode::Floating)).unwrap();
        }
        state.publish_scene(a, 1, 10).unwrap();
        state.publish_scene(b, 1, 30).unwrap();
        let p = Point::new(20., 20.).unwrap();
        let hits = |id, _| (id == a).then_some((1, HitRole::Input));
        state.pointer(p, None, 0, hits);
        state.publish_scene(a, 1, 11).unwrap();
        state.pointer(p, None, 0, hits);
        assert_eq!(
            state.take_event(a.context.session).unwrap().scene_revision,
            10
        );
        assert_eq!(
            state.take_event(a.context.session).unwrap().scene_revision,
            11
        );
        assert!(state.publish_scene(a, 1, 10).is_err());
        assert!(state.publish_scene(a, 2, 12).is_err());
        state
            .update(a, 1, 1, options(WindowMode::Floating))
            .unwrap();
        assert_eq!(state.get(a).unwrap().scene_revision, 11);
        assert_eq!(state.get(a).unwrap().revision, 2);
        state.revoke_owner(a.context.session);
        assert_eq!(state.get(b).unwrap().scene_revision, 30);
    }

    #[test]
    fn wheel_respects_transparency_and_modal_owner_without_stealing_focus() {
        let a = key(1, 1);
        let b = key(2, 1);
        let mut state = Windows::default();
        state.create(a, 1, options(WindowMode::Floating)).unwrap();
        state.create(b, 1, options(WindowMode::Floating)).unwrap();
        state.request_focus(b).unwrap();
        let p = Point::new(20., 20.).unwrap();
        assert!(!state.wheel(p, Scalar::ZERO, Scalar::ONE, 0, |_, _| None));
        assert!(state.wheel(p, Scalar::ZERO, Scalar::ONE, 0, |id, _| {
            (id == a).then_some((9, HitRole::Input))
        }));
        assert_eq!(state.focus(), Some(b));
        let event = state.take_event(a.context.session).unwrap();
        assert!(
            matches!(event.event, Event::Wheel { position, .. } if position == Point::new(10., 10.).unwrap())
        );
        state.close(a, DismissReason::Closed);
        state.create(a, 1, options(WindowMode::Modal)).unwrap();
        assert!(state.wheel(
            Point::new(900., 900.).unwrap(),
            Scalar::ZERO,
            Scalar::ONE,
            0,
            |_, _| None
        ));
        state.revoke_owner(a.context.session);
        assert!(!state.wheel(
            Point::new(900., 900.).unwrap(),
            Scalar::ZERO,
            Scalar::ONE,
            0,
            |_, _| None
        ));
        assert_eq!(state.focus(), Some(b));
    }
    #[test]
    fn modal_focus_and_owner_cleanup_are_isolated() {
        let a = key(1, 1);
        let b = key(2, 1);
        let mut s = Windows::default();
        s.create(b, 1, options(WindowMode::Floating)).unwrap();
        s.request_focus(b).unwrap();
        s.create(a, 1, options(WindowMode::Modal)).unwrap();
        assert!(s.request_focus(b).is_err());
        assert!(s.pointer(
            Point::new(500.0, 500.0).unwrap(),
            Some((1, true)),
            0,
            |_, _| None
        ));
        s.revoke_owner(a.context.session);
        assert!(s.get(a).is_none());
        assert!(s.get(b).is_some());
        assert_eq!(s.focus(), Some(b));
    }
    #[test]
    fn popup_dismissal_does_not_click_through() {
        let a = key(1, 1);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Popup)).unwrap();
        assert!(s.pointer(
            Point::new(0.0, 0.0).unwrap(),
            Some((1, true)),
            0,
            |_, _| panic!("dismissal must not hit-test underneath")
        ));
        assert!(s.get(a).is_none());
        assert!(s.pointer(
            Point::new(0.0, 0.0).unwrap(),
            Some((1, false)),
            0,
            |_, _| panic!("release must not click through")
        ));
        assert!(
            !s.pointer(Point::new(0.0, 0.0).unwrap(), Some((1, true)), 0, |_, _| {
                None
            })
        );
    }
    #[test]
    fn escape_dismisses_one_window_and_consumes_repeat_and_release() {
        let mut s = Windows::default();
        s.create(key(1, 1), 1, options(WindowMode::Modal)).unwrap();
        let mut popup = options(WindowMode::Popup);
        popup.parent = Some(key(1, 1));
        s.create(key(1, 2), 1, popup).unwrap();
        let key_event = |down, repeat| Event::Key {
            physical: 27,
            down,
            repeat,
            modifiers: 0,
        };
        assert!(s.keyboard(key_event(true, false), true));
        assert!(s.get(key(1, 2)).is_none());
        assert!(s.keyboard(key_event(true, true), true));
        assert!(s.keyboard(key_event(false, false), true));
        assert!(s.get(key(1, 1)).is_some());
    }
    #[test]
    fn terminal_click_releases_floating_keyboard_focus() {
        let mut s = Windows::default();
        s.create(key(1, 1), 1, options(WindowMode::Floating))
            .unwrap();
        s.request_focus(key(1, 1)).unwrap();
        assert!(!s.pointer(
            Point::new(400., 400.).unwrap(),
            Some((1, true)),
            0,
            |_, _| None
        ));
        assert_eq!(s.focus(), None);
        assert!(!s.keyboard(Event::Text("terminal".into()), false));
    }
    #[test]
    fn oversized_ime_revokes_only_its_owner() {
        let mut s = Windows::default();
        s.create(key(2, 1), 1, options(WindowMode::Floating))
            .unwrap();
        s.create(key(1, 1), 1, options(WindowMode::Modal)).unwrap();
        s.keyboard(
            Event::Ime {
                preedit: "a".repeat(MAX_EVENT_TEXT_BYTES + 1),
                selection: None,
            },
            false,
        );
        assert!(s.get(key(1, 1)).is_none());
        assert!(s.get(key(2, 1)).is_some());
    }
    #[test]
    fn rejected_popup_does_not_mutate_state() {
        let mut s = Windows::default();
        s.create(key(1, 1), 1, options(WindowMode::Modal)).unwrap();
        assert!(s.create(key(2, 1), 1, options(WindowMode::Popup)).is_err());
        assert!(s.get(key(2, 1)).is_none());
        assert_eq!(s.focus(), Some(key(1, 1)));
    }
    #[test]
    fn hiding_parent_cancels_child_capture_and_focus() {
        let mut s = Windows::default();
        let a = key(1, 1);
        let b = key(1, 2);
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        let mut popup = options(WindowMode::Popup);
        popup.parent = Some(a);
        s.create(b, 1, popup).unwrap();
        s.capture_pointer(b, Point::new(20.0, 20.0).unwrap())
            .unwrap();
        let mut hidden = options(WindowMode::Floating);
        hidden.visible = false;
        s.update(a, 1, 1, hidden).unwrap();
        assert_eq!(s.visible().count(), 0);
        assert!(s.capture.is_none());
        assert_eq!(s.focus(), None);
    }
    #[test]
    fn closing_popup_restores_terminal_if_floating_window_was_never_focused() {
        let mut s = Windows::default();
        s.create(key(1, 1), 1, options(WindowMode::Floating))
            .unwrap();
        s.create(key(1, 2), 1, options(WindowMode::Popup)).unwrap();
        s.close(key(1, 2), DismissReason::Closed);
        assert_eq!(s.focus(), None);
    }
    #[test]
    fn overflow_releases_only_the_unresponsive_owner() {
        let a = key(1, 1);
        let b = key(2, 1);
        let mut s = Windows::default();
        s.create(b, 1, options(WindowMode::Floating)).unwrap();
        s.create(a, 1, options(WindowMode::Modal)).unwrap();
        for _ in 0..MAX_PENDING_EVENTS + 1 {
            s.keyboard(
                Event::Key {
                    physical: 1,
                    down: true,
                    repeat: true,
                    modifiers: 0,
                },
                false,
            );
        }
        assert!(s.get(a).is_none());
        assert!(s.get(b).is_some());
    }
    #[test]
    fn dragging_advances_geometry_without_replacing_the_window() {
        let a = key(1, 1);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        s.pointer(
            Point::new(20.0, 20.0).unwrap(),
            Some((1, true)),
            0,
            |_, _| Some((1, HitRole::Drag)),
        );
        s.pointer(Point::new(35.0, 45.0).unwrap(), None, 0, |_, _| None);
        let w = s.get(a).unwrap();
        assert_eq!(w.options.bounds.origin, Point::new(25.0, 35.0).unwrap());
        assert_eq!(w.generation, 1);
    }
}
