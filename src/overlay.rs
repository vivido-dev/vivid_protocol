//! Pane-local window ownership, stacking, focus and gesture state for terminal overlays.
//!
//! Geometry and hit-test compilation are supplied by the presenter. This state machine never
//! performs OS input injection and never treats a surface ID without its owner as an identity.

use crate::identity::{SessionIdentity, SurfaceIdentity};
use crate::vector::{CursorShape, HitRegion, HitRole, InvalidScene, Point, Rect, Scalar};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

pub mod wire;

pub const MAX_WINDOWS: usize = 256;
pub const MAX_WINDOWS_PER_OWNER: usize = 32;
pub const MAX_PENDING_EVENTS: usize = 256;
/// The most nodes one overlay may publish in its semantic tree.
pub const MAX_SEMANTIC_NODES: usize = 256;
/// How deep a semantic tree may nest.
pub const MAX_SEMANTIC_DEPTH: usize = 32;
/// The most UTF-8 in one semantic node's label or value.
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 256;
/// How many actions one semantic node may advertise.
pub const MAX_SEMANTIC_ACTIONS: usize = 7;

/// What a semantic node is, in the vocabulary assistive technology understands.
///
/// A closed set rather than a platform role name: a host maps these onto whatever its toolkit
/// offers, and an unknown role must fail rather than silently become a generic group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SemanticRole {
    #[default]
    Generic,
    Application,
    Group,
    Heading,
    Text,
    Button,
    Switch,
    CheckBox,
    RadioButton,
    TextInput,
    Slider,
    SpinButton,
    ProgressIndicator,
    List,
    ListItem,
    Image,
    Link,
    Dialog,
    Tab,
    Separator,
}
impl SemanticRole {
    pub fn index(self) -> u64 {
        self as u64
    }
    pub fn from_index(index: u64) -> Option<Self> {
        const ALL: [SemanticRole; 20] = [
            SemanticRole::Generic,
            SemanticRole::Application,
            SemanticRole::Group,
            SemanticRole::Heading,
            SemanticRole::Text,
            SemanticRole::Button,
            SemanticRole::Switch,
            SemanticRole::CheckBox,
            SemanticRole::RadioButton,
            SemanticRole::TextInput,
            SemanticRole::Slider,
            SemanticRole::SpinButton,
            SemanticRole::ProgressIndicator,
            SemanticRole::List,
            SemanticRole::ListItem,
            SemanticRole::Image,
            SemanticRole::Link,
            SemanticRole::Dialog,
            SemanticRole::Tab,
            SemanticRole::Separator,
        ];
        ALL.get(usize::try_from(index).ok()?).copied()
    }
}

/// What assistive technology asked a semantic node to do.
///
/// Every action is payload-free. An action that would carry a value, such as setting a slider to
/// an arbitrary number, is deliberately absent: a host cannot synthesize one from a gesture, and
/// inventing a payload would be a guess about what the user meant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AccessibleAction {
    /// The node's default action, which is a click for a button and a focus for most else.
    #[default]
    Default,
    Focus,
    Click,
    Increment,
    Decrement,
    Expand,
    Collapse,
}
impl AccessibleAction {
    pub fn index(self) -> u64 {
        self as u64
    }
    pub fn from_index(index: u64) -> Option<Self> {
        // Seven actions, each with a counterpart in the toolkits a host builds on. A `Select`
        // action had no counterpart anywhere and was removed rather than advertised and then
        // ignored, and `Default` means the node can be activated at all.
        const ALL: [AccessibleAction; 7] = [
            AccessibleAction::Default,
            AccessibleAction::Focus,
            AccessibleAction::Click,
            AccessibleAction::Increment,
            AccessibleAction::Decrement,
            AccessibleAction::Expand,
            AccessibleAction::Collapse,
        ];
        ALL.get(usize::try_from(index).ok()?).copied()
    }
}

/// How a switch or check box reads to assistive technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggled {
    Off,
    On,
    Mixed,
}

/// One node of an application's semantic tree, as the application describes itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNode {
    /// Application-chosen identity, nonzero and unique within one tree.
    pub id: u64,
    pub role: SemanticRole,
    /// Window-local logical pixels, so assistive technology can point at what it describes.
    pub bounds: Rect,
    pub label: String,
    /// `[current, minimum, maximum]` for a control with a numeric value.
    pub numeric: Option<[Scalar; 3]>,
    /// A heading's level.
    pub level: Option<u8>,
    /// `[position, size]`, both one-based, for a node that is part of a set.
    pub set: Option<[u16; 2]>,
    pub toggled: Option<Toggled>,
    pub disabled: bool,
    pub actions: Vec<AccessibleAction>,
    /// Indices into the tree, not identities. A child's index is always greater than its
    /// parent's, which makes a cycle impossible by construction rather than by a visited set.
    pub children: Vec<u32>,
}

/// A complete application semantic tree for one scene revision.
///
/// Complete replacement rather than a delta, like a display list: a node the tree no longer names
/// stops existing, so a producer cannot leave a stale control behind for assistive technology to
/// offer a user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semantics {
    /// The published scene these nodes describe.
    pub scene_revision: u64,
    pub nodes: Vec<SemanticNode>,
}
impl Semantics {
    pub fn validate(&self) -> Result<(), InvalidScene> {
        if self.scene_revision == 0 {
            return Err(InvalidScene("semantics require a published scene revision"));
        }
        if self.nodes.is_empty() || self.nodes.len() > MAX_SEMANTIC_NODES {
            return Err(InvalidScene("semantic tree size is out of range"));
        }
        let mut identities = BTreeSet::new();
        for node in &self.nodes {
            if node.id == 0 || !identities.insert(node.id) {
                return Err(InvalidScene("semantic node IDs must be nonzero and unique"));
            }
            node.bounds.validate()?;
            if node.label.len() > MAX_SEMANTIC_TEXT_BYTES {
                return Err(InvalidScene("semantic node label exceeds its ceiling"));
            }
            if node.actions.len() > MAX_SEMANTIC_ACTIONS {
                return Err(InvalidScene("semantic node lists too many actions"));
            }
            if node
                .set
                .is_some_and(|[position, size]| position == 0 || size == 0 || position > size)
            {
                return Err(InvalidScene(
                    "semantic set position must be within its size",
                ));
            }
        }
        // Every node but the root is a child exactly once, and a child always sits later in the
        // list than its parent. Together these make the list a tree and make a cycle impossible.
        let mut claimed = vec![false; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            let mut seen = BTreeSet::new();
            for child in &node.children {
                let child = usize::try_from(*child)
                    .map_err(|_| InvalidScene("semantic child index is out of range"))?;
                if child <= index || child >= self.nodes.len() || !seen.insert(child) {
                    return Err(InvalidScene(
                        "semantic children must follow their parent and be listed once",
                    ));
                }
                if std::mem::replace(&mut claimed[child], true) {
                    return Err(InvalidScene("a semantic node has two parents"));
                }
            }
        }
        if claimed[1..].iter().any(|claimed| !claimed) {
            return Err(InvalidScene("a semantic node is unreachable from the root"));
        }
        // Index ordering bounds nothing on its own: a chain of 256 nodes is 256 deep.
        let mut depth = vec![0_u32; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            for child in &node.children {
                let child = *child as usize;
                depth[child] = depth[index] + 1;
                if depth[child] as usize > MAX_SEMANTIC_DEPTH {
                    return Err(InvalidScene("semantic tree is too deep"));
                }
            }
        }
        Ok(())
    }
}

/// A press sequence longer than this restarts at one, exactly as a fourth terminal click does.
pub const MAX_CLICKS: u8 = 3;
/// The most UTF-8 an overlay may copy out in one gesture.
pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;
/// A host MUST NOT accept a clipboard write more than this long after the gesture that caused
/// it. The exact interval is the host's, but a producer cannot ask for an unbounded one.
pub const MAX_CLIPBOARD_GESTURE_AGE: Duration = Duration::from_secs(2);
pub const MAX_EVENT_TEXT_BYTES: usize = 4096;
pub const MAX_EVENT_QUEUE_BYTES: usize = 128 * 1024;
pub const MAX_OWNERS: usize = 16;

/// Normative overlay modifier bits. A presenter MUST NOT set a reserved bit, and a receiver
/// MUST reject an event that does. Platform bitmasks are translated, never forwarded.
pub mod modifiers {
    pub const SHIFT: u32 = 1;
    pub const CONTROL: u32 = 1 << 1;
    pub const ALT: u32 = 1 << 2;
    pub const SUPER: u32 = 1 << 3;
    pub const CAPS_LOCK: u32 = 1 << 4;
    pub const NUM_LOCK: u32 = 1 << 5;
    pub const KNOWN_MASK: u32 = SHIFT | CONTROL | ALT | SUPER | CAPS_LOCK | NUM_LOCK;
}

/// Normative overlay pointer buttons, sharing `desktop-surface-v1` section 7's assignment so a
/// producer can accept either lane's events without knowing which presenter produced them.
pub mod buttons {
    pub const PRIMARY: u16 = 0;
    pub const AUXILIARY: u16 = 1;
    pub const SECONDARY: u16 = 2;
    pub const BACK: u16 = 3;
    pub const FORWARD: u16 = 4;
    /// Additional device buttons occupy 5..=MAXIMUM. The range is bounded so a hostile or broken
    /// device cannot force a producer to size per-button state from an untrusted number.
    pub const MAXIMUM: u16 = 31;
}

/// Physical keys are USB HID keyboard-page usages, as in `desktop-surface-v1` section 7.
pub mod keys {
    pub const UNMAPPED: u32 = 0;
    pub const FIRST_USAGE: u32 = 0x04;
    pub const LAST_USAGE: u32 = 0xe7;
    /// Zero reports a physical key the keyboard page does not name; it is not an identity.
    pub fn valid(usage: u32) -> bool {
        usage == UNMAPPED || (FIRST_USAGE..=LAST_USAGE).contains(&usage)
    }
}

/// One wheel or trackpad delta together with the device detail a producer needs to interpret it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    pub dx: Scalar,
    pub dy: Scalar,
    /// True for pixel-precise devices such as trackpads, false for detented wheels.
    pub precise: bool,
    pub phase: ScrollPhase,
}

/// Whether a wheel delta came from a pixel-precise device and where it sits in a gesture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScrollPhase {
    /// A device that reports no gesture boundaries, such as an ordinary detented wheel.
    #[default]
    None,
    Began,
    Changed,
    Ended,
    Cancelled,
}

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
        /// 1-3 for a press, 0 for motion or a release (overlay-pointer-v1). A host counts clicks
        /// from its own double-click policy; a producer never has to reproduce that timing.
        clicks: u8,
        /// Normalized 0-1 from a device that reports pressure, absent when it reported none.
        /// A host MUST NOT invent a value for hardware that has no such sensor.
        pressure: Option<Scalar>,
    },
    /// Assistive technology invoked one semantic node (overlay-a11y-v1).
    Accessibility {
        node: u64,
        action: AccessibleAction,
    },
    /// The pointer entered or left one region (overlay-pointer-v1).
    ///
    /// Emitted whenever the hovered region changes, including when a new scene removes the
    /// region under the pointer, so a producer can drive hover styling without diffing pointer
    /// events and without missing a transition it never saw.
    Hover {
        /// The region being entered or left. Zero is the default window rectangle.
        region: u64,
        entered: bool,
    },
    Wheel {
        position: Point,
        dx: Scalar,
        dy: Scalar,
        modifiers: u32,
        /// True for pixel-precise devices such as trackpads, false for detented wheels.
        precise: bool,
        phase: ScrollPhase,
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

/// One pointer report: where it is, what changed, and what the device measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerReport {
    pub position: Point,
    pub button: Option<(u16, bool)>,
    pub modifiers: u32,
    /// 1-3 for a press, 0 for motion or a release.
    pub clicks: u8,
    /// Absent when the device reported no pressure rather than reporting zero.
    pub pressure: Option<Scalar>,
}

#[derive(Debug, Clone, Copy)]
struct Capture {
    window: SurfaceIdentity,
    role: HitRole,
    start: Point,
    bounds: Rect,
    region: u64,
    /// The captured region's cursor, so a drag keeps the shape it started with.
    cursor: Option<CursorShape>,
}

/// The region the pointer currently sits in. One pointer means one hover at a time, even with
/// several windows open.
#[derive(Debug, Clone, Copy)]
struct Hover {
    window: SurfaceIdentity,
    region: u64,
    cursor: Option<CursorShape>,
}

#[derive(Debug, Default)]
pub struct Windows {
    windows: BTreeMap<SurfaceIdentity, Window>,
    stack: Vec<SurfaceIdentity>,
    focus: Option<SurfaceIdentity>,
    hover: Option<Hover>,
    focus_history: Vec<SurfaceIdentity>,
    swallowed_buttons: BTreeSet<u16>,
    swallowed_escape: bool,
    capture: Option<Capture>,
    events: BTreeMap<SessionIdentity, VecDeque<WindowEvent>>,
    overflowed: BTreeSet<SessionIdentity>,
}

impl Windows {
    pub fn get(&self, id: SurfaceIdentity) -> Option<&Window> {
        self.windows.get(&id)
    }
    pub fn focus(&self) -> Option<SurfaceIdentity> {
        self.focus
    }
    pub fn has_pointer_capture(&self) -> bool {
        self.capture.is_some()
    }
    /// Native pane focus loss cancels gestures and held input without destroying window state.
    /// Focus restoration follows the same modal eligibility rules as closing a child window.
    pub fn set_pane_focus(&mut self, focused: bool) {
        if focused {
            self.restore_focus();
        } else {
            if let Some(capture) = self.capture.take() {
                self.emit(capture.window, Event::Cancel);
            }
            self.change_focus(None);
            self.swallowed_buttons.clear();
            self.swallowed_escape = false;
        }
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
        if self.overflowed.contains(&id.context.session) {
            return Err(InvalidScene("overlay owner input was revoked"));
        }
        if !self.events.contains_key(&id.context.session)
            && self.events.len() + self.overflowed.len() >= MAX_OWNERS
        {
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
        if self.hover.is_some_and(|hover| hover.window == id) {
            // The region is going away, so the leave is emitted while its window still exists.
            self.set_hover(None);
        }
        if self.focus == Some(id) {
            self.change_focus(None);
        }
        self.emit(id, Event::Dismissed(reason));
        self.windows.remove(&id);
        self.stack.retain(|key| *key != id);
        self.focus_history.retain(|key| *key != id);
    }
    pub fn revoke_owner(&mut self, owner: SessionIdentity) {
        self.overflowed.remove(&owner);
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
            self.overflowed.insert(id.context.session);
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
            self.overflowed.insert(id.context.session);
            return;
        }
        queue.push_back(e);
    }
    /// Report a fail-closed input overflow so the host also retires the authenticated lane.
    pub fn take_overflow(&mut self, owner: SessionIdentity) -> bool {
        self.overflowed.remove(&owner)
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
            cursor: self
                .hover
                .filter(|hover| hover.window == id)
                .and_then(|hover| hover.cursor),
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
        scroll: Scroll,
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
                    dx: scroll.dx,
                    dy: scroll.dy,
                    modifiers,
                    precise: scroll.precise,
                    phase: scroll.phase,
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
        report: PointerReport,
        hit: impl Fn(SurfaceIdentity, Point) -> Option<HitRegion>,
    ) -> bool {
        let PointerReport {
            position,
            button,
            modifiers,
            clicks,
            pressure,
        } = report;
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
                        clicks,
                        pressure,
                    },
                );
            }
            if button.is_some_and(|(_, down)| !down) {
                self.capture = None;
                // A drag deliberately does not retrigger hover as it passes over regions, so
                // the release is where the pointer's hover is finally re-evaluated.
                self.refresh_hover(position, &hit);
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
        let Some((id, local, target)) = self.target_at(position, &hit) else {
            // Leaving every window is a hover transition like any other.
            self.set_hover(None);
            let blocked = self.top_modal().is_some();
            if !blocked && button.is_some_and(|(_, down)| down) {
                self.change_focus(None);
                self.focus_history.clear();
            }
            return blocked;
        };
        if button.is_some_and(|(_, down)| down) {
            let _ = self.request_focus(id);
            if matches!(target.role, HitRole::Drag | HitRole::Resize(_))
                && let Some(w) = self.windows.get(&id)
            {
                self.capture = Some(Capture {
                    window: id,
                    role: target.role,
                    start: position,
                    bounds: w.options.bounds,
                    region: target.id,
                    cursor: target.cursor,
                });
            }
        }
        self.set_hover(Some(Hover {
            window: id,
            region: target.id,
            cursor: target.cursor,
        }));
        self.emit(
            id,
            Event::Pointer {
                position: local,
                region: target.id,
                button,
                modifiers,
                clicks,
                pressure,
            },
        );
        true
    }

    /// The window, window-local point and region under a viewport point.
    fn target_at(
        &self,
        position: Point,
        hit: &impl Fn(SurfaceIdentity, Point) -> Option<HitRegion>,
    ) -> Option<(SurfaceIdentity, Point, HitRegion)> {
        self.stack
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
                let target = hit(id, local)?;
                (target.role != HitRole::Transparent).then_some((id, local, target))
            })
    }

    /// Re-evaluate hover after the scene under the pointer changed.
    ///
    /// A new scene can add, remove, or reshape the region beneath a stationary pointer, and a
    /// producer must not have to synthesise the resulting leave or enter from a later motion.
    pub fn refresh_hover(
        &mut self,
        position: Point,
        hit: &impl Fn(SurfaceIdentity, Point) -> Option<HitRegion>,
    ) {
        match self.target_at(position, hit) {
            Some((id, _, target)) => self.set_hover(Some(Hover {
                window: id,
                region: target.id,
                cursor: target.cursor,
            })),
            None => self.set_hover(None),
        }
    }

    fn set_hover(&mut self, next: Option<Hover>) {
        let same = match (self.hover, next) {
            (None, None) => true,
            (Some(a), Some(b)) => a.window == b.window && a.region == b.region,
            _ => false,
        };
        if same {
            // The region is unchanged, but a replacement scene may have restyled it.
            self.hover = next;
            return;
        }
        if let Some(previous) = self.hover {
            self.emit(
                previous.window,
                Event::Hover {
                    region: previous.region,
                    entered: false,
                },
            );
        }
        self.hover = next;
        if let Some(current) = next {
            self.emit(
                current.window,
                Event::Hover {
                    region: current.region,
                    entered: true,
                },
            );
        }
    }

    /// The window the pointer currently sits in, if any.
    pub fn hovered_window(&self) -> Option<SurfaceIdentity> {
        self.hover.map(|hover| hover.window)
    }

    /// The cursor the pointer calls for: a captured region keeps the shape it started with.
    pub fn cursor(&self) -> Option<CursorShape> {
        self.capture
            .and_then(|capture| capture.cursor)
            .or_else(|| self.hover.and_then(|hover| hover.cursor))
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
        Event::Pointer {
            button,
            modifiers,
            clicks,
            pressure,
            ..
        } => {
            modifiers & !modifiers::KNOWN_MASK == 0
                && button.is_none_or(|(button, _)| button <= buttons::MAXIMUM)
                && *clicks <= MAX_CLICKS
                && pressure.is_none_or(|p| p >= Scalar::ZERO && p <= Scalar::ONE)
        }
        Event::Wheel { modifiers, .. } => modifiers & !modifiers::KNOWN_MASK == 0,
        Event::Key {
            physical,
            modifiers,
            ..
        } => modifiers & !modifiers::KNOWN_MASK == 0 && keys::valid(*physical),
        // Zero is not an identity, so an action naming one cannot be dispatched.
        Event::Accessibility { node, .. } => *node != 0,
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
    #[test]
    fn pane_focus_loss_cancels_capture_and_overflow_is_observable_once() {
        let mut windows = Windows::default();
        let id = key(1, 1);
        let neighbor = key(2, 1);
        windows
            .create(id, 1, options(WindowMode::Floating))
            .unwrap();
        windows
            .create(neighbor, 1, options(WindowMode::Floating))
            .unwrap();
        windows.request_focus(id).unwrap();
        windows
            .capture_pointer(id, Point::new(1., 1.).unwrap())
            .unwrap();
        windows.set_pane_focus(false);
        assert!(windows.capture.is_none());
        assert_eq!(windows.focus(), None);
        windows.set_pane_focus(true);
        assert_eq!(windows.focus(), Some(id));
        windows.keyboard(Event::Text("x".repeat(MAX_EVENT_TEXT_BYTES + 1)), false);
        assert!(windows.get(id).is_none());
        assert!(windows.get(neighbor).is_some());
        assert!(windows.take_overflow(id.context.session));
        assert!(!windows.take_overflow(id.context.session));
    }
    #[test]
    fn capture_records_the_presenter_supplied_origin() {
        // The presenter establishes capture from its own last observed pointer position, so the
        // stored origin must be exactly what it passed: gestures measure their delta against it.
        let mut windows = Windows::default();
        let id = key(1, 1);
        windows
            .create(id, 1, options(WindowMode::Floating))
            .unwrap();
        windows.request_focus(id).unwrap();
        let origin = Point::new(37.5, 12.25).unwrap();
        windows.capture_pointer(id, origin).unwrap();
        assert_eq!(windows.capture.map(|c| c.start), Some(origin));
        windows.release_pointer(id);
        assert!(!windows.has_pointer_capture());
    }
    /// A plain report at one point, with no button, click count, or pressure.
    fn at(x: f64, y: f64) -> PointerReport {
        PointerReport {
            position: Point::new(x, y).unwrap(),
            button: None,
            modifiers: 0,
            clicks: 0,
            pressure: None,
        }
    }
    /// A press of the primary button at one point.
    fn press(x: f64, y: f64) -> PointerReport {
        PointerReport {
            button: Some((0, true)),
            ..at(x, y)
        }
    }
    fn region(id: u64, role: HitRole) -> HitRegion {
        HitRegion {
            id,
            role,
            cursor: None,
        }
    }
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
        let hits = |id, _| (id == a).then_some(region(1, HitRole::Input));
        state.pointer(at(20., 20.), hits);
        state.publish_scene(a, 1, 11).unwrap();
        state.pointer(at(20., 20.), hits);
        // The enter is stamped with the scene it was decided against, like the motion beside it.
        let enter = state.take_event(a.context.session).unwrap();
        assert!(matches!(
            enter.event,
            Event::Hover {
                region: 1,
                entered: true
            }
        ));
        assert_eq!(enter.scene_revision, 10);
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
        let scroll = Scroll {
            dx: Scalar::ZERO,
            dy: Scalar::ONE,
            precise: true,
            phase: ScrollPhase::Changed,
        };
        assert!(!state.wheel(p, scroll, 0, |_, _| None));
        assert!(state.wheel(p, scroll, 0, |id, _| {
            (id == a).then_some((9, HitRole::Input))
        }));
        assert_eq!(state.focus(), Some(b));
        let event = state.take_event(a.context.session).unwrap();
        // Device detail reaches the producer beside the delta, not folded into it.
        assert!(matches!(
            event.event,
            Event::Wheel { position, precise: true, phase: ScrollPhase::Changed, .. }
                if position == Point::new(10., 10.).unwrap()
        ));
        state.close(a, DismissReason::Closed);
        state.create(a, 1, options(WindowMode::Modal)).unwrap();
        let outside = Point::new(900., 900.).unwrap();
        assert!(state.wheel(outside, scroll, 0, |_, _| None));
        state.revoke_owner(a.context.session);
        assert!(!state.wheel(outside, scroll, 0, |_, _| None));
        assert_eq!(state.focus(), Some(b));
    }
    /// Drain one owner's events as a compact script, so an ordering assertion reads as the
    /// sequence a producer would actually act on.
    fn script(s: &mut Windows, owner: SessionIdentity) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(event) = s.take_event(owner) {
            out.push(match event.event {
                Event::Hover { region, entered } => {
                    format!("{} {region}", if entered { "enter" } else { "leave" })
                }
                Event::Pointer { region, button, .. } => match button {
                    Some((_, true)) => format!("press {region}"),
                    Some((_, false)) => format!("release {region}"),
                    None => format!("move {region}"),
                },
                Event::Dismissed(_) => "dismissed".to_owned(),
                Event::Focus(true) => "focus on".to_owned(),
                Event::Focus(false) => "focus off".to_owned(),
                Event::Geometry { settled: true, .. } => "geometry settled".to_owned(),
                Event::Geometry { settled: false, .. } => "geometry".to_owned(),
                other => format!("{other:?}"),
            });
        }
        out
    }

    #[test]
    fn hover_emits_one_transition_per_change() {
        let a = key(1, 1);
        let b = key(1, 2);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        let mut second = options(WindowMode::Floating);
        second.bounds = Rect::new(300., 300., 100., 80.).unwrap();
        s.create(b, 1, second).unwrap();
        // Two regions inside the first window, so a move between them is a transition too.
        let inside_a = |id, point: Point| {
            (id == a).then(|| {
                if point.x.get() < 50. {
                    region(1, HitRole::Input)
                } else {
                    region(2, HitRole::Input)
                }
            })
        };
        let mut moves = Vec::new();
        for (x, y) in [
            (10., 10.),
            // Moving to the second region is a transition even though the window is the same.
            (60., 10.),
            // Re-reporting the same position changes nothing.
            (60., 10.),
            // Leaving every window is a leave like any other.
            (900., 900.),
            (10., 10.),
            (60., 10.),
            (900., 900.),
        ] {
            s.pointer(at(x, y), inside_a);
            moves.extend(script(&mut s, a.context.session));
        }
        assert_eq!(
            moves,
            vec![
                "enter 1", "move 1", "leave 1", "enter 2", "move 2", "move 2", "leave 2",
                "enter 1", "move 1", "leave 1", "enter 2", "move 2", "leave 2",
            ]
        );

        // Crossing into another window leaves the first before entering the second.
        let cross = |id, point: Point| {
            if id == a {
                return Some(region(1, HitRole::Input));
            }
            (id == b && point.x.get() < 50.).then_some(region(9, HitRole::Input))
        };
        s.pointer(at(10., 10.), cross);
        s.pointer(at(310., 310.), cross);
        assert_eq!(
            script(&mut s, a.context.session),
            vec!["enter 1", "move 1", "leave 1", "enter 9", "move 9"]
        );
    }

    #[test]
    fn a_replacement_scene_retires_the_hovered_region() {
        // A new scene can remove the region under a stationary pointer; the producer must not
        // have to wait for the next motion to learn that it is no longer hovered.
        let a = key(1, 1);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        let p = Point::new(20., 20.).unwrap();
        s.pointer(at(20., 20.), |_, _| Some(region(1, HitRole::Input)));
        assert_eq!(script(&mut s, a.context.session), vec!["enter 1", "move 1"]);

        // The replacement declares nothing under the pointer.
        s.refresh_hover(p, &|_, _| None);
        assert_eq!(script(&mut s, a.context.session), vec!["leave 1"]);
        // Re-evaluating the same state again emits nothing.
        s.refresh_hover(p, &|_, _| None);
        assert!(s.take_event(a.context.session).is_none());
    }

    #[test]
    fn a_captured_region_keeps_its_cursor_after_the_pointer_leaves_it() {
        let a = key(1, 1);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        // A right-edge resize keeps the window's origin still, so the pointer can genuinely
        // leave the region mid-gesture; a drag would carry the region along with it.
        let edge = |id, point: Point| {
            (id == a && point.x.get() < 50.).then_some(HitRegion {
                id: 4,
                role: HitRole::Resize(2),
                cursor: Some(CursorShape::ResizeRight),
            })
        };
        s.pointer(press(20., 20.), edge);
        assert_eq!(s.cursor(), Some(CursorShape::ResizeRight));
        assert_eq!(
            script(&mut s, a.context.session),
            vec!["focus on", "enter 4", "press 4"]
        );

        // Past the region's right edge: the gesture must not revert the shape it started with.
        s.pointer(at(95., 20.), edge);
        assert_eq!(s.cursor(), Some(CursorShape::ResizeRight));
        assert_eq!(script(&mut s, a.context.session), vec!["geometry"]);

        // The release ends the gesture, and that is where hover is re-evaluated.
        s.pointer(
            PointerReport {
                button: Some((0, false)),
                ..at(95., 20.)
            },
            edge,
        );
        assert_eq!(s.cursor(), None);
        assert_eq!(
            script(&mut s, a.context.session),
            vec!["geometry settled", "leave 4"]
        );
    }

    #[test]
    fn closing_a_window_ends_its_hover() {
        let a = key(1, 1);
        let mut s = Windows::default();
        s.create(a, 1, options(WindowMode::Floating)).unwrap();
        s.pointer(at(20., 20.), |_, _| Some(region(1, HitRole::Input)));
        assert_eq!(script(&mut s, a.context.session), vec!["enter 1", "move 1"]);
        s.close(a, DismissReason::Closed);
        // The leave precedes the dismissal, so a producer styling on hover clears it in order.
        assert_eq!(
            script(&mut s, a.context.session),
            vec!["leave 1", "dismissed"]
        );
    }

    /// A node whose identity mirrors its index, so a test reads as a tree shape. The root is
    /// index 0 with identity 1, because a node ID of zero is not an identity.
    fn node(index: usize, role: SemanticRole, children: Vec<u32>) -> SemanticNode {
        SemanticNode {
            id: index as u64 + 1,
            role,
            bounds: Rect::new(0., 0., 10., 10.).unwrap(),
            label: String::new(),
            numeric: None,
            level: None,
            set: None,
            toggled: None,
            disabled: false,
            actions: Vec::new(),
            children,
        }
    }
    fn tree(nodes: Vec<SemanticNode>) -> Semantics {
        Semantics {
            scene_revision: 1,
            nodes,
        }
    }

    #[test]
    fn a_well_formed_semantic_tree_validates() {
        let mut heading = node(1, SemanticRole::Heading, vec![2]);
        heading.level = Some(2);
        let mut button = node(2, SemanticRole::Button, Vec::new());
        button.label = "Press me".to_owned();
        button.actions = vec![AccessibleAction::Click, AccessibleAction::Focus];
        tree(vec![node(0, SemanticRole::Group, vec![1]), heading, button])
            .validate()
            .unwrap();
    }

    #[test]
    fn a_malformed_semantic_tree_is_refused_rather_than_walked() {
        for nodes in [
            // A node that parents itself would let a host walk in a cycle.
            vec![node(0, SemanticRole::Group, vec![0])],
            // A child that precedes its parent would too.
            vec![
                node(0, SemanticRole::Group, vec![1, 2]),
                node(1, SemanticRole::Group, vec![2]),
                node(2, SemanticRole::Group, Vec::new()),
            ],
            // A child listed twice.
            vec![
                node(0, SemanticRole::Group, vec![1, 1]),
                node(1, SemanticRole::Group, Vec::new()),
            ],
            // Two parents claim the same child.
            vec![
                node(0, SemanticRole::Group, vec![1, 2]),
                node(1, SemanticRole::Group, vec![2]),
                node(2, SemanticRole::Group, Vec::new()),
            ],
            // A node nobody parents is unreachable from the root.
            vec![
                node(0, SemanticRole::Group, vec![1]),
                node(1, SemanticRole::Group, Vec::new()),
                node(2, SemanticRole::Group, Vec::new()),
            ],
            // A child index past the end.
            vec![node(0, SemanticRole::Group, vec![9])],
        ] {
            assert!(tree(nodes).validate().is_err());
        }

        // A repeated identity, which is a different failure from a repeated child index.
        let mut repeated = node(1, SemanticRole::Group, Vec::new());
        repeated.id = 1;
        assert!(
            tree(vec![node(0, SemanticRole::Group, vec![1]), repeated])
                .validate()
                .is_err()
        );

        // A set position outside its size.
        let mut broken = node(1, SemanticRole::ListItem, Vec::new());
        broken.set = Some([3, 2]);
        assert!(
            tree(vec![node(0, SemanticRole::List, vec![1]), broken])
                .validate()
                .is_err()
        );

        // A label past its ceiling.
        let mut wordy = node(1, SemanticRole::Text, Vec::new());
        wordy.label = "x".repeat(MAX_SEMANTIC_TEXT_BYTES + 1);
        assert!(
            tree(vec![node(0, SemanticRole::Group, vec![1]), wordy])
                .validate()
                .is_err()
        );
    }

    #[test]
    fn index_ordering_alone_does_not_bound_depth() {
        // A chain of 256 nodes satisfies "a child follows its parent" and is still 256 deep, so
        // depth needs its own ceiling rather than being implied by the ordering rule.
        let chain = MAX_SEMANTIC_DEPTH + 2;
        let nodes = (0..chain)
            .map(|index| {
                let children = if index + 1 < chain {
                    vec![(index + 1) as u32]
                } else {
                    Vec::new()
                };
                node(index, SemanticRole::Group, children)
            })
            .collect();
        let error = tree(nodes).validate().unwrap_err();
        assert!(format!("{error:?}").contains("deep"), "{error:?}");
    }

    #[test]
    fn semantic_roles_and_actions_are_closed_bounded_sets() {
        for role in [
            SemanticRole::Generic,
            SemanticRole::Application,
            SemanticRole::Group,
            SemanticRole::Heading,
            SemanticRole::Text,
            SemanticRole::Button,
            SemanticRole::Switch,
            SemanticRole::CheckBox,
            SemanticRole::RadioButton,
            SemanticRole::TextInput,
            SemanticRole::Slider,
            SemanticRole::SpinButton,
            SemanticRole::ProgressIndicator,
            SemanticRole::List,
            SemanticRole::ListItem,
            SemanticRole::Image,
            SemanticRole::Link,
            SemanticRole::Dialog,
            SemanticRole::Tab,
            SemanticRole::Separator,
        ] {
            assert_eq!(SemanticRole::from_index(role.index()), Some(role));
        }
        assert_eq!(SemanticRole::from_index(20), None);
        for action in [
            AccessibleAction::Default,
            AccessibleAction::Focus,
            AccessibleAction::Click,
            AccessibleAction::Increment,
            AccessibleAction::Decrement,
            AccessibleAction::Expand,
            AccessibleAction::Collapse,
        ] {
            assert_eq!(AccessibleAction::from_index(action.index()), Some(action));
        }
        // The set is closed: one past the end is not an action.
        assert_eq!(AccessibleAction::from_index(7), None);
    }

    #[test]
    fn cursor_shapes_are_a_closed_bounded_set() {
        for shape in [
            CursorShape::Default,
            CursorShape::Pointer,
            CursorShape::Text,
            CursorShape::Move,
            CursorShape::Crosshair,
            CursorShape::NotAllowed,
            CursorShape::Grab,
            CursorShape::Grabbing,
            CursorShape::Wait,
            CursorShape::Progress,
            CursorShape::ResizeLeft,
            CursorShape::ResizeRight,
            CursorShape::ResizeUp,
            CursorShape::ResizeDown,
            CursorShape::ResizeUpLeft,
            CursorShape::ResizeUpRight,
            CursorShape::ResizeDownLeft,
            CursorShape::ResizeDownRight,
            CursorShape::ResizeLeftRight,
            CursorShape::ResizeUpDown,
        ] {
            assert_eq!(CursorShape::from_index(shape.index()), Some(shape));
        }
        assert_eq!(CursorShape::from_index(20), None);
        assert_eq!(CursorShape::from_index(u64::MAX), None);
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
        assert!(s.pointer(press(500., 500.), |_, _| None));
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
        assert!(s.pointer(press(0., 0.), |_, _| panic!(
            "dismissal must not hit-test underneath"
        )));
        assert!(s.get(a).is_none());
        assert!(s.pointer(
            PointerReport {
                button: Some((0, false)),
                ..at(0., 0.)
            },
            |_, _| panic!("release must not click through")
        ));
        assert!(!s.pointer(press(0., 0.), |_, _| None));
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
        assert!(!s.pointer(press(400., 400.), |_, _| None));
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
        s.pointer(press(20., 20.), |_, _| Some(region(1, HitRole::Drag)));
        s.pointer(at(35., 45.), |_, _| None);
        let w = s.get(a).unwrap();
        assert_eq!(w.options.bounds.origin, Point::new(25.0, 35.0).unwrap());
        assert_eq!(w.generation, 1);
    }
}
