//! Bounded session reconciliation pages for core control queries.
//!
//! Pages contain flat owner-relative summaries so a surface with many tracks
//! cannot force an oversized indivisible entry. Cursors are opaque to producers;
//! presenters bind them to the complete session identity and one revision.

// Rust guideline compliant 2026-10-07
use crate::{
    cbor::Value,
    identity::SessionIdentity,
    messages::{MessageError, PayloadMap, StrictMap, invalid_value, validate_header_object},
    revision::SessionRevision,
};
use sha2::{Digest as _, Sha256};

/// Maximum summaries in one reconciliation page.
pub const MAX_ENTRIES: usize = 256;
/// Default summaries requested when the query omits its limit.
pub const DEFAULT_ENTRIES: u16 = 64;
/// Maximum opaque continuation cursor size.
pub const MAX_CURSOR_BYTES: usize = 64;

/// A bounded `QUERY_SESSION` payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// An opaque cursor returned by the previous page, if any.
    pub cursor: Option<Vec<u8>>,
    /// Maximum entries, from one through 256.
    pub maximum_entries: u16,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            cursor: None,
            maximum_entries: DEFAULT_ENTRIES,
        }
    }
}
impl Query {
    /// Validates and encodes the query payload.
    ///
    /// # Errors
    /// Returns an error for an empty/oversized cursor or invalid entry limit.
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        validate_limit(self.maximum_entries)?;
        validate_cursor(self.cursor.as_deref())?;
        let mut out = vec![(1, Value::Unsigned(u64::from(self.maximum_entries)))];
        if let Some(cursor) = &self.cursor {
            out.insert(0, (0, Value::Bytes(cursor.clone())));
        }
        Ok(out)
    }
    /// Decodes a query before allocating any cursor storage.
    ///
    /// # Errors
    /// Returns an error for unknown fields, invalid limits, cursors or object IDs.
    pub fn decode(object_id: u64, value: &Value) -> Result<Self, MessageError> {
        validate_header_object(object_id, 0)?;
        let map = StrictMap::new("QUERY_SESSION", value, &[0, 1])?;
        let maximum_entries =
            u16::try_from(map.optional_u64(1)?.unwrap_or(u64::from(DEFAULT_ENTRIES)))
                .map_err(|_| invalid_value("QUERY_SESSION", 0, "entry limit is too large"))?;
        validate_limit(maximum_entries)?;
        let cursor = map.optional_bytes(0)?;
        validate_cursor(cursor)?;
        Ok(Self {
            cursor: cursor.map(<[u8]>::to_vec),
            maximum_entries,
        })
    }
}

fn validate_limit(value: u16) -> Result<(), MessageError> {
    if value == 0 || usize::from(value) > MAX_ENTRIES {
        return Err(invalid_value(
            "QUERY_SESSION",
            0,
            "entry limit must be in 1..=256",
        ));
    }
    Ok(())
}
fn validate_cursor(value: Option<&[u8]>) -> Result<(), MessageError> {
    if value.is_some_and(|v| v.is_empty() || v.len() > MAX_CURSOR_BYTES) {
        return Err(invalid_value(
            "SESSION_STATUS",
            0,
            "invalid continuation cursor length",
        ));
    }
    Ok(())
}

/// Object classes in a reconciliation page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Kind {
    /// Context revision; only the context ID is nonzero.
    Context = 0,
    /// Surface revision and generation; object ID is zero.
    Surface = 1,
    /// Track revision and channel generation; object ID is the track ID.
    Track = 2,
    /// Active slot; object ID is the slot and detail is the active track ID.
    Slot = 3,
}

/// One fixed-size owner-relative object summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Summary {
    /// The kind of retained object.
    pub kind: Kind,
    /// Nonzero owning context ID.
    pub context_id: u64,
    /// Owning surface ID, or zero for a context.
    pub surface_id: u64,
    /// Track ID or slot number; zero for a context or surface.
    pub object_id: u64,
    /// The object's revision; slots use the owning surface revision.
    pub revision: u64,
    /// Surface/channel generation, or zero for a context.
    pub generation: u64,
    /// Native context/surface/track lifecycle, or zero for an active slot.
    pub state: u64,
    /// Track milestones or active track ID; zero for contexts and surfaces.
    pub detail: u64,
}
impl Summary {
    /// Validates and encodes the summary map.
    ///
    /// # Errors
    /// Returns an error for incomplete or contradictory object identity.
    pub fn to_value(self) -> Result<Value, MessageError> {
        self.validate()?;
        Ok(Value::Map(vec![
            (0, Value::Unsigned(self.kind as u64)),
            (1, Value::Unsigned(self.context_id)),
            (2, Value::Unsigned(self.surface_id)),
            (3, Value::Unsigned(self.object_id)),
            (4, Value::Unsigned(self.revision)),
            (5, Value::Unsigned(self.generation)),
            (6, Value::Unsigned(self.state)),
            (7, Value::Unsigned(self.detail)),
        ]))
    }
    fn validate(self) -> Result<(), MessageError> {
        let valid = self.context_id != 0
            && self.revision != 0
            && match self.kind {
                Kind::Context => {
                    self.surface_id == 0
                        && self.object_id == 0
                        && self.generation == 0
                        && self.detail == 0
                }
                Kind::Surface => {
                    self.surface_id != 0
                        && self.object_id == 0
                        && self.generation != 0
                        && self.detail == 0
                }
                Kind::Track => self.surface_id != 0 && self.object_id != 0 && self.generation != 0,
                Kind::Slot => {
                    self.surface_id != 0
                        && self.object_id != 0
                        && self.generation != 0
                        && self.state == 0
                        && self.detail != 0
                }
            };
        if !valid {
            return Err(invalid_value(
                "SESSION_STATUS",
                0,
                "invalid summary identity",
            ));
        }
        Ok(())
    }
    /// Decodes one complete summary without retaining untrusted extensions.
    ///
    /// # Errors
    /// Returns an error for unknown fields, kinds or invalid object identity.
    pub fn decode(value: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("SESSION_STATUS summary", value, &[0, 1, 2, 3, 4, 5, 6, 7])?;
        let kind = match map.required_u64(0)? {
            0 => Kind::Context,
            1 => Kind::Surface,
            2 => Kind::Track,
            3 => Kind::Slot,
            _ => return Err(invalid_value("SESSION_STATUS", 0, "unknown summary kind")),
        };
        let out = Self {
            kind,
            context_id: map.required_u64(1)?,
            surface_id: map.required_u64(2)?,
            object_id: map.required_u64(3)?,
            revision: map.required_u64(4)?,
            generation: map.required_u64(5)?,
            state: map.required_u64(6)?,
            detail: map.required_u64(7)?,
        };
        out.validate()?;
        Ok(out)
    }
}

/// Session status fields for a presenter snapshot or bounded response.
///
/// Presenters may supply a larger unencoded snapshot to [`paginate`]. Encoding
/// or decoding an individual wire page always enforces [`MAX_ENTRIES`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// Monotonic revision shared by every page of this snapshot.
    pub session_revision: u64,
    /// Current scene revision, queried separately for individual nodes.
    pub scene_revision: u64,
    /// Current target generation.
    pub target_generation: u64,
    /// Active (1), suspended (2), or closing (3).
    pub establishment_state: u64,
    /// Current lease resume generation, or zero for a root session.
    pub resume_generation: u64,
    /// Whether input is disabled; this observation never grants input authority.
    pub input_disabled: bool,
    /// Ordered summaries; encoded pages admit at most 256 objects.
    pub entries: Vec<Summary>,
    /// Continuation token, absent on the final page.
    pub cursor: Option<Vec<u8>>,
    /// Surface, track and scene-node counts, respectively.
    pub usage: [u64; 3],
}
impl Page {
    /// Validates and encodes one response payload.
    ///
    /// # Errors
    /// Returns an error for oversized pages, invalid revisions, identities or cursors.
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        if self.session_revision == 0
            || self.target_generation == 0
            || !(1..=3).contains(&self.establishment_state)
            || self.entries.len() > MAX_ENTRIES
        {
            return Err(invalid_value(
                "SESSION_STATUS",
                0,
                "invalid page extent or revision",
            ));
        }
        validate_cursor(self.cursor.as_deref())?;
        validate_entries(&self.entries, self.cursor.is_some())?;
        let mut payload = vec![
            (0, Value::Unsigned(self.session_revision)),
            (1, Value::Unsigned(self.scene_revision)),
            (2, Value::Unsigned(self.target_generation)),
            (3, Value::Unsigned(self.establishment_state)),
            (4, Value::Unsigned(self.resume_generation)),
            (5, Value::Bool(self.input_disabled)),
            (
                6,
                Value::Array(
                    self.entries
                        .iter()
                        .copied()
                        .map(Summary::to_value)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                8,
                Value::Map(vec![
                    (0, Value::Unsigned(self.usage[0])),
                    (1, Value::Unsigned(self.usage[1])),
                    (2, Value::Unsigned(self.usage[2])),
                ]),
            ),
        ];
        if let Some(cursor) = &self.cursor {
            payload.insert(7, (7, Value::Bytes(cursor.clone())));
        }
        Ok(payload)
    }
    /// Decodes a bounded response before allocating its summary vector.
    ///
    /// # Errors
    /// Returns an error for invalid fields, revisions, limits or object identities.
    pub fn decode(object_id: u64, value: &Value) -> Result<Self, MessageError> {
        validate_header_object(object_id, 0)?;
        let map = StrictMap::new("SESSION_STATUS", value, &[0, 1, 2, 3, 4, 5, 6, 7, 8])?;
        let Value::Array(entries) = map.required(6)? else {
            return Err(invalid_value(
                "SESSION_STATUS",
                0,
                "entries must be an array",
            ));
        };
        if entries.len() > MAX_ENTRIES {
            return Err(invalid_value("SESSION_STATUS", 0, "too many entries"));
        }
        let usage = StrictMap::new("SESSION_STATUS usage", map.required(8)?, &[0, 1, 2])?;
        let cursor = map.optional_bytes(7)?;
        validate_cursor(cursor)?;
        let page = Self {
            session_revision: map.required_u64(0)?,
            scene_revision: map.required_u64(1)?,
            target_generation: map.required_u64(2)?,
            establishment_state: map.required_u64(3)?,
            resume_generation: map.required_u64(4)?,
            input_disabled: map.required_bool(5)?,
            entries: entries
                .iter()
                .map(Summary::decode)
                .collect::<Result<_, _>>()?,
            cursor: cursor.map(<[u8]>::to_vec),
            usage: [
                usage.required_u64(0)?,
                usage.required_u64(1)?,
                usage.required_u64(2)?,
            ],
        };
        if page.session_revision == 0
            || page.target_generation == 0
            || !(1..=3).contains(&page.establishment_state)
        {
            return Err(invalid_value(
                "SESSION_STATUS",
                0,
                "invalid page revision or establishment state",
            ));
        }
        validate_entries(&page.entries, page.cursor.is_some())?;
        Ok(page)
    }
}

/// An owner-local revision clock for reconciliation snapshots.
///
/// Presenters advance this before publishing successful control mutations.
/// Observing the sorted fixed-size summaries also detects asynchronous changes.
#[derive(Clone, Debug)]
pub struct Revision {
    value: SessionRevision,
    fingerprint: Option<[u8; 32]>,
}
impl Default for Revision {
    fn default() -> Self {
        Self {
            value: SessionRevision::ONE,
            fingerprint: None,
        }
    }
}
impl Revision {
    /// Creates a clock from a confirmed welcome revision.
    ///
    /// # Errors
    /// Returns an error for a zero revision.
    pub fn new(value: u64) -> Result<Self, MessageError> {
        let value = SessionRevision::new(value)
            .require_nonzero()
            .map_err(|_| invalid_value("SESSION_STATUS", 0, "zero session revision"))?;
        Ok(Self {
            value,
            fingerprint: None,
        })
    }
    /// Returns the current monotonic revision.
    pub fn get(&self) -> u64 {
        self.value.get()
    }
    /// Checks capacity before a control mutation changes retained state.
    ///
    /// # Errors
    /// Returns an error when the revision is exhausted.
    pub fn check_advance(&self) -> Result<(), MessageError> {
        self.value
            .advance()
            .map(|_| ())
            .map_err(|_| invalid_value("SESSION_STATUS", 0, "session revision exhausted"))
    }
    /// Advances after a successful, previously checked control mutation.
    ///
    /// # Errors
    /// Returns an error instead of wrapping an exhausted revision.
    pub fn advance(&mut self) -> Result<(), MessageError> {
        self.value = self
            .value
            .advance()
            .map_err(|_| invalid_value("SESSION_STATUS", 0, "session revision exhausted"))?;
        self.fingerprint = None;
        Ok(())
    }
    /// Observes current snapshot contents and returns their monotonic revision.
    ///
    /// The caller bounds snapshot storage by its existing object admission.
    /// This method hashes fixed-size summaries without allocating another copy.
    ///
    /// # Errors
    /// Returns an error for invalid or unsorted identities or revision exhaustion.
    pub fn observe(&mut self, snapshot: &Page) -> Result<u64, MessageError> {
        let mut digest = Sha256::new();
        for value in [
            snapshot.scene_revision,
            snapshot.target_generation,
            snapshot.establishment_state,
            snapshot.resume_generation,
            u64::from(snapshot.input_disabled),
            snapshot.usage[0],
            snapshot.usage[1],
            snapshot.usage[2],
        ] {
            digest.update(value.to_be_bytes());
        }
        let mut previous = None;
        for entry in &snapshot.entries {
            entry.validate()?;
            let key = (
                entry.kind,
                entry.context_id,
                entry.surface_id,
                entry.object_id,
            );
            if previous.is_some_and(|old| old >= key) {
                return Err(invalid_value(
                    "SESSION_STATUS",
                    6,
                    "summaries are not strictly ordered",
                ));
            }
            previous = Some(key);
            for value in [
                entry.kind as u64,
                entry.context_id,
                entry.surface_id,
                entry.object_id,
                entry.revision,
                entry.generation,
                entry.state,
                entry.detail,
            ] {
                digest.update(value.to_be_bytes());
            }
        }
        let fingerprint = digest.finalize().into();
        if self.fingerprint.is_some_and(|old| old != fingerprint) {
            self.advance()?;
        }
        self.fingerprint = Some(fingerprint);
        Ok(self.value.get())
    }
}

fn validate_entries(entries: &[Summary], continuation: bool) -> Result<(), MessageError> {
    if continuation && entries.is_empty() {
        return Err(invalid_value(
            "SESSION_STATUS",
            6,
            "continuation page makes no progress",
        ));
    }
    if entries.windows(2).any(|pair| {
        let a = pair[0];
        let b = pair[1];
        (a.kind, a.context_id, a.surface_id, a.object_id)
            >= (b.kind, b.context_id, b.surface_id, b.object_id)
    }) {
        return Err(invalid_value(
            "SESSION_STATUS",
            6,
            "duplicate or unordered summaries",
        ));
    }
    Ok(())
}

/// Why a presenter cannot return a reconciliation page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageError {
    /// The cursor is malformed, foreign or outside its snapshot.
    InvalidCursor,
    /// The owner changed after the cursor was issued.
    RevisionChanged,
    /// Even one entry cannot fit the negotiated control body.
    BodyLimit,
    /// The presenter supplied inconsistent snapshot metadata.
    InvalidSnapshot,
}
impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidCursor => "invalid session status cursor",
            Self::RevisionChanged => "session status revision changed",
            Self::BodyLimit => "session status exceeds negotiated control body",
            Self::InvalidSnapshot => "invalid session status snapshot",
        })
    }
}
impl std::error::Error for PageError {}

/// Selects one page, bounded by both entry count and encoded control-body size.
///
/// Cursors carry presenter/session identity, revision and offset, never authority.
/// The current owner always selects the snapshot; cursor bytes cannot select it.
///
/// # Errors
/// Returns [`PageError`] for stale/foreign cursors, invalid snapshots or tiny bodies.
pub fn paginate(
    snapshot: &Page,
    owner: SessionIdentity,
    query: &Query,
    request_id: u64,
    maximum_body: u32,
) -> Result<Page, PageError> {
    validate_limit(query.maximum_entries).map_err(|_| PageError::InvalidCursor)?;
    let mut prefix = Vec::with_capacity(40);
    prefix.extend_from_slice(&owner.presenter().0);
    prefix.extend_from_slice(&owner.session_id().to_be_bytes());
    prefix.extend_from_slice(&snapshot.session_revision.to_be_bytes());
    let offset = if let Some(cursor) = &query.cursor {
        if cursor.len() != 40 || cursor[..24] != prefix[..24] {
            return Err(PageError::InvalidCursor);
        }
        if cursor[24..32] != prefix[24..32] {
            return Err(PageError::RevisionChanged);
        }
        let bytes = cursor[32..40]
            .try_into()
            .map_err(|_| PageError::InvalidCursor)?;
        let offset =
            usize::try_from(u64::from_be_bytes(bytes)).map_err(|_| PageError::InvalidCursor)?;
        if offset == 0 || offset >= snapshot.entries.len() {
            return Err(PageError::InvalidCursor);
        }
        offset
    } else {
        0
    };
    let mut end = offset + usize::from(query.maximum_entries).min(snapshot.entries.len() - offset);
    loop {
        let mut page = Page {
            entries: snapshot.entries[offset..end].to_vec(),
            cursor: None,
            session_revision: snapshot.session_revision,
            scene_revision: snapshot.scene_revision,
            target_generation: snapshot.target_generation,
            establishment_state: snapshot.establishment_state,
            resume_generation: snapshot.resume_generation,
            input_disabled: snapshot.input_disabled,
            usage: snapshot.usage,
        };
        if end < snapshot.entries.len() {
            let mut cursor = prefix.clone();
            cursor.extend_from_slice(
                &u64::try_from(end)
                    .map_err(|_| PageError::InvalidSnapshot)?
                    .to_be_bytes(),
            );
            page.cursor = Some(cursor);
        }
        let payload = page.payload().map_err(|_| PageError::InvalidSnapshot)?;
        let bytes = crate::messages::Envelope::new(request_id, payload)
            .encode()
            .map_err(|_| PageError::InvalidSnapshot)?;
        if bytes.len() <= maximum_body as usize {
            return Ok(page);
        }
        if end - offset <= 1 {
            return Err(PageError::BodyLimit);
        }
        end = offset + (end - offset) / 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{identity::PresenterInstanceId, messages::Envelope};

    fn owner(id: u64) -> SessionIdentity {
        SessionIdentity::new(PresenterInstanceId([7; 16]), id).unwrap()
    }
    fn snapshot(count: u64) -> Page {
        Page {
            session_revision: 1,
            scene_revision: 0,
            target_generation: 1,
            establishment_state: 1,
            resume_generation: 0,
            input_disabled: true,
            entries: (1..=count)
                .map(|id| Summary {
                    kind: Kind::Surface,
                    context_id: 1,
                    surface_id: id,
                    object_id: 0,
                    revision: 1,
                    generation: 1,
                    state: 1,
                    detail: 0,
                })
                .collect(),
            cursor: None,
            usage: [count, 0, 0],
        }
    }
    #[test]
    fn bounded_pages_walk_every_entry_without_truncating_large_snapshots() {
        let snapshot = snapshot(700);
        let mut query = Query {
            maximum_entries: 37,
            ..Default::default()
        };
        let mut found = Vec::new();
        loop {
            let page = paginate(&snapshot, owner(1), &query, 7, 4096).unwrap();
            let payload = page.payload().unwrap();
            assert_eq!(Page::decode(0, &Value::Map(payload)).unwrap(), page);
            assert!(page.entries.len() <= 37);
            assert_eq!(page.usage, snapshot.usage);
            found.extend(page.entries.iter().copied());
            let Some(cursor) = page.cursor else {
                break;
            };
            query.cursor = Some(cursor);
        }
        assert_eq!(found, snapshot.entries);
    }
    #[test]
    fn negotiated_body_limit_reduces_pages_and_never_emits_a_nonprogress_cursor() {
        let snapshot = snapshot(100);
        let page = paginate(&snapshot, owner(1), &Query::default(), 9, 200).unwrap();
        assert!(!page.entries.is_empty() && page.entries.len() < 64);
        assert!(
            Envelope::new(9, page.payload().unwrap())
                .encode()
                .unwrap()
                .len()
                <= 200
        );
        assert_eq!(
            paginate(&snapshot, owner(1), &Query::default(), 9, 1),
            Err(PageError::BodyLimit)
        );
        let mut empty = snapshot;
        empty.entries.clear();
        assert!(
            paginate(&empty, owner(1), &Query::default(), 9, 200)
                .unwrap()
                .cursor
                .is_none()
        );
        empty.cursor = Some(vec![1]);
        empty.payload().unwrap_err();
    }
    #[test]
    fn cursors_are_qualified_by_presenter_owner_revision_and_valid_offset() {
        let mut snapshot = snapshot(4);
        let first = paginate(
            &snapshot,
            owner(1),
            &Query {
                maximum_entries: 1,
                ..Default::default()
            },
            1,
            4096,
        )
        .unwrap();
        let mut query = Query {
            cursor: first.cursor,
            ..Default::default()
        };
        assert_eq!(
            paginate(&snapshot, owner(2), &query, 1, 4096),
            Err(PageError::InvalidCursor)
        );
        let foreign = SessionIdentity::new(PresenterInstanceId([8; 16]), 1).unwrap();
        assert_eq!(
            paginate(&snapshot, foreign, &query, 1, 4096),
            Err(PageError::InvalidCursor)
        );
        snapshot.session_revision += 1;
        assert_eq!(
            paginate(&snapshot, owner(1), &query, 1, 4096),
            Err(PageError::RevisionChanged)
        );
        snapshot.session_revision -= 1;
        query.cursor.as_mut().unwrap()[32..].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            paginate(&snapshot, owner(1), &query, 1, 4096),
            Err(PageError::InvalidCursor)
        );
    }
    #[test]
    fn clocks_cover_async_changes_delete_recreate_and_exhaustion() {
        let mut snapshot = snapshot(2);
        let mut clock = Revision::default();
        assert_eq!(clock.observe(&snapshot).unwrap(), 1);
        assert_eq!(clock.observe(&snapshot).unwrap(), 1);
        snapshot.entries[0].revision += 1;
        assert_eq!(clock.observe(&snapshot).unwrap(), 2);
        clock.check_advance().unwrap();
        clock.advance().unwrap();
        assert_eq!(
            clock.observe(&snapshot).unwrap(),
            3,
            "identical rows after delete/recreate still advance"
        );
        let mut exhausted = Revision::new(u64::MAX).unwrap();
        exhausted.observe(&snapshot).unwrap();
        snapshot.resume_generation += 1;
        exhausted.observe(&snapshot).unwrap_err();
        assert!(exhausted.check_advance().is_err());
        assert_eq!(exhausted.get(), u64::MAX);
    }
    #[test]
    fn query_and_page_decoders_reject_unbounded_and_ambiguous_payloads() {
        assert_eq!(
            Query::decode(0, &Value::Map(vec![])).unwrap(),
            Query::default()
        );
        for limit in [0, 257, u64::MAX] {
            Query::decode(0, &Value::Map(vec![(1, Value::Unsigned(limit))])).unwrap_err();
        }
        for length in [0, MAX_CURSOR_BYTES + 1] {
            Query::decode(0, &Value::Map(vec![(0, Value::Bytes(vec![0; length]))])).unwrap_err();
        }
        Query::decode(7, &Value::Map(vec![])).unwrap_err();
        Query::decode(0, &Value::Map(vec![(2, Value::Unsigned(1))])).unwrap_err();
        let mut page = snapshot(2);
        page.entries[1] = page.entries[0];
        page.payload().unwrap_err();
        page.entries = snapshot(257).entries;
        page.payload().unwrap_err();
        let mut payload = snapshot(1).payload().unwrap();
        payload.iter_mut().find(|field| field.0 == 6).unwrap().1 =
            Value::Array(vec![
                snapshot(1).entries[0].to_value().unwrap();
                MAX_ENTRIES + 1
            ]);
        Page::decode(0, &Value::Map(payload)).unwrap_err();
    }
    #[test]
    fn summaries_preserve_full_context_surface_track_and_slot_identities() {
        let root = Summary {
            kind: Kind::Context,
            context_id: 7,
            surface_id: 0,
            object_id: 0,
            revision: 4,
            generation: 0,
            state: 1,
            detail: 0,
        };
        let surface = Summary {
            kind: Kind::Surface,
            surface_id: 3,
            generation: 2,
            ..root
        };
        let track = Summary {
            kind: Kind::Track,
            object_id: 8,
            generation: 5,
            detail: 16,
            ..surface
        };
        let slot = Summary {
            kind: Kind::Slot,
            object_id: 1,
            detail: 8,
            state: 0,
            ..surface
        };
        for summary in [root, surface, track, slot] {
            assert_eq!(
                Summary::decode(&summary.to_value().unwrap()).unwrap(),
                summary
            );
        }
        Summary {
            context_id: 0,
            ..track
        }
        .to_value()
        .unwrap_err();
        Summary {
            surface_id: 0,
            ..track
        }
        .to_value()
        .unwrap_err();
        Summary {
            generation: 0,
            ..track
        }
        .to_value()
        .unwrap_err();
        Summary { detail: 0, ..slot }.to_value().unwrap_err();
    }
}
