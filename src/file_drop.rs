//! Consent-gated presenter-to-producer regular-file transfer schemas.

use std::io;

use crate::{
    HARD_MAX_RECORD_BODY,
    cbor::{self, Value},
    messages::{
        MessageError, PayloadMap, StrictMap, invalid_value, require_nonzero, validate_header_object,
    },
    revision::{FileDropEpoch, FileDropGrantGeneration, FileTransferGeneration, SurfaceGeneration},
};

pub const FILE_DATA_PREFIX_SIZE: usize = 16;
pub const FILE_TRANSFER_NONCE_BYTES: usize = 16;
pub const FILE_TRANSFER_TAG_BYTES: usize = 16;
pub const FILE_HASH_BYTES: usize = 32;
pub const MAX_FILE_DROP_NAME_BYTES: usize = 255;
pub const MAX_COMMITTED_PATH_BYTES: usize = 4096;
pub const MAX_PENDING_FILE_DROPS: u64 = 16;
pub const MAX_ACTIVE_FILE_TRANSFERS: u64 = 4;
pub const DEFAULT_PENDING_FILE_DROPS: u64 = 4;
pub const DEFAULT_ACTIVE_FILE_TRANSFERS: u64 = 1;
pub const MIN_FILE_DROP_DEADLINE_US: u64 = 1_000_000;
pub const MAX_FILE_DROP_DEADLINE_US: u64 = 300_000_000;
pub const DEFAULT_FILE_DROP_ACCEPTANCE_US: u64 = 30_000_000;
pub const DEFAULT_FILE_TRANSFER_IDLE_US: u64 = 60_000_000;
pub const FILE_DROP_RESULT_RETENTION_US: u64 = 60_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FileDropDestination {
    ShellCwd = 1,
    DesktopFolder = 2,
}

impl TryFrom<u64> for FileDropDestination {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::ShellCwd),
            2 => Ok(Self::DesktopFolder),
            _ => Err(invalid_value(
                "file-drop destination",
                4,
                "is not a known destination",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FileDropBindingState {
    Disabled = 0,
    Enabled = 1,
    Denied = 2,
    Standby = 3,
}

impl TryFrom<u64> for FileDropBindingState {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::Enabled),
            2 => Ok(Self::Denied),
            3 => Ok(Self::Standby),
            _ => Err(invalid_value(
                "FILE_DROP_BOUND",
                5,
                "has an unknown binding state",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDropBinding {
    pub producer_epoch: FileDropEpoch,
    pub context_id: u64,
    pub surface_id: u64,
    pub surface_generation: SurfaceGeneration,
    pub destination: Option<FileDropDestination>,
    pub maximum_file_bytes: u64,
    pub maximum_pending_offers: u64,
    pub maximum_active_transfers: u64,
    pub maximum_record_body: u32,
    pub acceptance_timeout_us: u64,
    pub idle_timeout_us: u64,
}

impl FileDropBinding {
    pub fn disabled(&self) -> bool {
        self.destination.is_none()
    }

    pub fn validate(&self, header_object_id: u64) -> Result<(), MessageError> {
        self.producer_epoch
            .require_nonzero()
            .map_err(|_| invalid_value("SET_FILE_DROP_BINDING", 0, "must be nonzero"))?;
        validate_header_object(header_object_id, self.surface_id)?;
        if self.disabled() {
            require_nonzero("SET_FILE_DROP_BINDING", 1, self.context_id)?;
            if (self.surface_id == 0 && self.surface_generation != SurfaceGeneration::ZERO)
                || (self.surface_id != 0 && self.surface_generation.require_nonzero().is_err())
            {
                return Err(invalid_value(
                    "SET_FILE_DROP_BINDING",
                    3,
                    "does not match the scoped surface identity",
                ));
            }
            if self.maximum_file_bytes != 0
                || self.maximum_pending_offers != 0
                || self.maximum_active_transfers != 0
                || self.maximum_record_body != 0
                || self.acceptance_timeout_us != 0
                || self.idle_timeout_us != 0
            {
                return Err(invalid_value(
                    "SET_FILE_DROP_BINDING",
                    5,
                    "has nonzero limits while disabled",
                ));
            }
            return Ok(());
        }
        require_nonzero("SET_FILE_DROP_BINDING", 1, self.context_id)?;
        if self.surface_id == 0 {
            if self.surface_generation != SurfaceGeneration::ZERO {
                return Err(invalid_value(
                    "SET_FILE_DROP_BINDING",
                    3,
                    "must be zero for a target-wide binding",
                ));
            }
        } else {
            self.surface_generation.require_nonzero().map_err(|_| {
                invalid_value("SET_FILE_DROP_BINDING", 3, "must be nonzero for a surface")
            })?;
        }
        if self.destination.is_none() || self.maximum_file_bytes == 0 {
            return Err(invalid_value(
                "SET_FILE_DROP_BINDING",
                4,
                "requires a destination and nonzero file limit",
            ));
        }
        validate_binding_limits(
            self.maximum_pending_offers,
            self.maximum_active_transfers,
            self.maximum_record_body,
            self.acceptance_timeout_us,
            self.idle_timeout_us,
            "SET_FILE_DROP_BINDING",
        )
    }

    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        self.validate(self.surface_id)?;
        Ok(vec![
            (0, Value::Unsigned(self.producer_epoch.get())),
            (1, Value::Unsigned(self.context_id)),
            (2, Value::Unsigned(self.surface_id)),
            (3, Value::Unsigned(self.surface_generation.get())),
            (
                4,
                Value::Unsigned(self.destination.map_or(0, |value| value as u64)),
            ),
            (5, Value::Unsigned(self.maximum_file_bytes)),
            (6, Value::Unsigned(self.maximum_pending_offers)),
            (7, Value::Unsigned(self.maximum_active_transfers)),
            (8, Value::Unsigned(u64::from(self.maximum_record_body))),
            (9, Value::Unsigned(self.acceptance_timeout_us)),
            (10, Value::Unsigned(self.idle_timeout_us)),
        ])
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "SET_FILE_DROP_BINDING",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        )?;
        let destination = match map.required_u64(4)? {
            0 => None,
            value => Some(value.try_into()?),
        };
        let binding = Self {
            producer_epoch: FileDropEpoch::new(map.required_u64(0)?),
            context_id: map.required_u64(1)?,
            surface_id: map.required_u64(2)?,
            surface_generation: SurfaceGeneration::new(map.required_u64(3)?),
            destination,
            maximum_file_bytes: map.required_u64(5)?,
            maximum_pending_offers: map.required_u64(6)?,
            maximum_active_transfers: map.required_u64(7)?,
            maximum_record_body: map.required_u32(8)?,
            acceptance_timeout_us: map.required_u64(9)?,
            idle_timeout_us: map.required_u64(10)?,
        };
        binding.validate(header_object_id)?;
        Ok(binding)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDropGrant {
    pub producer_epoch: FileDropEpoch,
    pub grant_generation: FileDropGrantGeneration,
    pub context_id: u64,
    pub surface_id: u64,
    pub surface_generation: SurfaceGeneration,
    pub state: FileDropBindingState,
    pub destination: Option<FileDropDestination>,
    pub maximum_file_bytes: u64,
    pub maximum_pending_offers: u64,
    pub maximum_active_transfers: u64,
    pub maximum_record_body: u32,
    pub acceptance_timeout_us: u64,
    pub idle_timeout_us: u64,
    pub reason: u64,
}

impl FileDropGrant {
    pub fn payload(self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.producer_epoch.get())),
            (1, Value::Unsigned(self.grant_generation.get())),
            (2, Value::Unsigned(self.context_id)),
            (3, Value::Unsigned(self.surface_id)),
            (4, Value::Unsigned(self.surface_generation.get())),
            (5, Value::Unsigned(self.state as u64)),
            (
                6,
                Value::Unsigned(self.destination.map_or(0, |value| value as u64)),
            ),
            (7, Value::Unsigned(self.maximum_file_bytes)),
            (8, Value::Unsigned(self.maximum_pending_offers)),
            (9, Value::Unsigned(self.maximum_active_transfers)),
            (10, Value::Unsigned(u64::from(self.maximum_record_body))),
            (11, Value::Unsigned(self.acceptance_timeout_us)),
            (12, Value::Unsigned(self.idle_timeout_us)),
            (13, Value::Unsigned(self.reason)),
        ]
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "FILE_DROP_BOUND",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
        )?;
        let destination = match map.required_u64(6)? {
            0 => None,
            value => Some(value.try_into()?),
        };
        let grant = Self {
            producer_epoch: FileDropEpoch::new(map.required_u64(0)?),
            grant_generation: FileDropGrantGeneration::new(map.required_u64(1)?),
            context_id: map.required_u64(2)?,
            surface_id: map.required_u64(3)?,
            surface_generation: SurfaceGeneration::new(map.required_u64(4)?),
            state: map.required_u64(5)?.try_into()?,
            destination,
            maximum_file_bytes: map.required_u64(7)?,
            maximum_pending_offers: map.required_u64(8)?,
            maximum_active_transfers: map.required_u64(9)?,
            maximum_record_body: map.required_u32(10)?,
            acceptance_timeout_us: map.required_u64(11)?,
            idle_timeout_us: map.required_u64(12)?,
            reason: map.required_u64(13)?,
        };
        validate_header_object(header_object_id, grant.surface_id)?;
        grant
            .producer_epoch
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_DROP_BOUND", 0, "must be nonzero"))?;
        if (grant.surface_id == 0 && grant.surface_generation != SurfaceGeneration::ZERO)
            || (grant.surface_id != 0 && grant.surface_generation.require_nonzero().is_err())
        {
            return Err(invalid_value(
                "FILE_DROP_BOUND",
                4,
                "does not match the scoped surface identity",
            ));
        }
        if grant.state != FileDropBindingState::Disabled {
            grant
                .grant_generation
                .require_nonzero()
                .map_err(|_| invalid_value("FILE_DROP_BOUND", 1, "must be nonzero"))?;
            require_nonzero("FILE_DROP_BOUND", 2, grant.context_id)?;
            if grant.destination.is_none() || grant.maximum_file_bytes == 0 {
                return Err(invalid_value(
                    "FILE_DROP_BOUND",
                    6,
                    "enabled grant has no effective destination or file limit",
                ));
            }
            validate_binding_limits(
                grant.maximum_pending_offers,
                grant.maximum_active_transfers,
                grant.maximum_record_body,
                grant.acceptance_timeout_us,
                grant.idle_timeout_us,
                "FILE_DROP_BOUND",
            )?;
        } else if grant.state == FileDropBindingState::Disabled {
            require_nonzero("FILE_DROP_BOUND", 2, grant.context_id)?;
            if grant.grant_generation != FileDropGrantGeneration::ZERO
                || grant.destination.is_some()
                || grant.maximum_file_bytes != 0
                || grant.maximum_pending_offers != 0
                || grant.maximum_active_transfers != 0
                || grant.maximum_record_body != 0
                || grant.acceptance_timeout_us != 0
                || grant.idle_timeout_us != 0
            {
                return Err(invalid_value(
                    "FILE_DROP_BOUND",
                    1,
                    "disabled binding retains a live grant or limits",
                ));
            }
        }
        Ok(grant)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDropTuple {
    pub producer_epoch: FileDropEpoch,
    pub grant_generation: FileDropGrantGeneration,
    pub context_id: u64,
    pub surface_id: u64,
    pub surface_generation: SurfaceGeneration,
    pub drop_id: u64,
}

impl FileDropTuple {
    fn payload(self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.producer_epoch.get())),
            (1, Value::Unsigned(self.grant_generation.get())),
            (2, Value::Unsigned(self.context_id)),
            (3, Value::Unsigned(self.surface_id)),
            (4, Value::Unsigned(self.surface_generation.get())),
            (5, Value::Unsigned(self.drop_id)),
        ]
    }

    fn decode(schema: &'static str, map: &StrictMap<'_>) -> Result<Self, MessageError> {
        let tuple = Self {
            producer_epoch: FileDropEpoch::new(map.required_u64(0)?),
            grant_generation: FileDropGrantGeneration::new(map.required_u64(1)?),
            context_id: require_nonzero(schema, 2, map.required_u64(2)?)?,
            surface_id: map.required_u64(3)?,
            surface_generation: SurfaceGeneration::new(map.required_u64(4)?),
            drop_id: require_nonzero(schema, 5, map.required_u64(5)?)?,
        };
        tuple
            .producer_epoch
            .require_nonzero()
            .map_err(|_| invalid_value(schema, 0, "must be nonzero"))?;
        tuple
            .grant_generation
            .require_nonzero()
            .map_err(|_| invalid_value(schema, 1, "must be nonzero"))?;
        if tuple.surface_id == 0 {
            if tuple.surface_generation != SurfaceGeneration::ZERO {
                return Err(invalid_value(
                    schema,
                    4,
                    "must be zero for a target-wide drop",
                ));
            }
        } else {
            tuple
                .surface_generation
                .require_nonzero()
                .map_err(|_| invalid_value(schema, 4, "must be nonzero for a surface drop"))?;
        }
        Ok(tuple)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDropOffer {
    pub binding: FileDropTuple,
    pub suggested_name: String,
    pub declared_length: u64,
}

impl FileDropOffer {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        validate_suggested_name(&self.suggested_name)?;
        let mut payload = self.binding.payload();
        payload.push((6, Value::Text(self.suggested_name.clone())));
        payload.push((7, Value::Unsigned(self.declared_length)));
        Ok(payload)
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("FILE_DROP_OFFER", payload, &[0, 1, 2, 3, 4, 5, 6, 7])?;
        let offer = Self {
            binding: FileDropTuple::decode("FILE_DROP_OFFER", &map)?,
            suggested_name: map.required_text(6)?.to_owned(),
            declared_length: map.required_u64(7)?,
        };
        validate_header_object(header_object_id, offer.binding.drop_id)?;
        validate_suggested_name(&offer.suggested_name)?;
        Ok(offer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptFileDrop {
    pub binding: FileDropTuple,
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub maximum_record_body: u32,
    pub initial_maximum_body_bytes: u64,
    pub initial_maximum_records: u64,
}

impl AcceptFileDrop {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.validate(self.binding.drop_id)?;
        let mut payload = self.binding.payload();
        payload.extend([
            (6, Value::Unsigned(self.transfer_id)),
            (7, Value::Unsigned(self.transfer_generation.get())),
            (8, Value::Unsigned(u64::from(self.maximum_record_body))),
            (9, Value::Unsigned(self.initial_maximum_body_bytes)),
            (10, Value::Unsigned(self.initial_maximum_records)),
        ]);
        Ok(payload)
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "ACCEPT_FILE_DROP",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        )?;
        let accepted = Self {
            binding: FileDropTuple::decode("ACCEPT_FILE_DROP", &map)?,
            transfer_id: map.required_u64(6)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(7)?),
            maximum_record_body: map.required_u32(8)?,
            initial_maximum_body_bytes: map.required_u64(9)?,
            initial_maximum_records: map.required_u64(10)?,
        };
        accepted.validate(header_object_id)?;
        Ok(accepted)
    }

    fn validate(self, header_object_id: u64) -> Result<(), MessageError> {
        validate_header_object(header_object_id, self.binding.drop_id)?;
        require_nonzero("ACCEPT_FILE_DROP", 6, self.transfer_id)?;
        if self.transfer_generation != FileTransferGeneration::ONE {
            return Err(invalid_value(
                "ACCEPT_FILE_DROP",
                7,
                "initial generation must be one",
            ));
        }
        validate_credit(
            self.maximum_record_body,
            self.initial_maximum_body_bytes,
            self.initial_maximum_records,
            "ACCEPT_FILE_DROP",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileDropAccepted {
    pub drop_id: u64,
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub open_timeout_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelFileDrop {
    pub binding: FileDropTuple,
    pub reason: u64,
}

impl CancelFileDrop {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        if self.reason > 7 {
            return Err(invalid_value(
                "CANCEL_FILE_DROP",
                6,
                "has an unknown cancellation reason",
            ));
        }
        let mut payload = self.binding.payload();
        payload.push((6, Value::Unsigned(self.reason)));
        Ok(payload)
    }

    pub fn decode(
        schema: &'static str,
        header_object_id: u64,
        payload: &Value,
    ) -> Result<Self, MessageError> {
        let map = StrictMap::new(schema, payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let cancelled = Self {
            binding: FileDropTuple::decode(schema, &map)?,
            reason: map.required_u64(6)?,
        };
        validate_header_object(header_object_id, cancelled.binding.drop_id)?;
        if cancelled.reason > 7 {
            return Err(invalid_value(
                schema,
                6,
                "has an unknown cancellation reason",
            ));
        }
        Ok(cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvanceFileTransfer {
    pub context_id: u64,
    pub surface_id: u64,
    pub drop_id: u64,
    pub transfer_id: u64,
    pub expected_generation: FileTransferGeneration,
    pub new_generation: FileTransferGeneration,
    pub committed_offset: u64,
    pub maximum_body_bytes: u64,
    pub maximum_records: u64,
}

impl AdvanceFileTransfer {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        self.validate(self.transfer_id)?;
        Ok(vec![
            (0, Value::Unsigned(self.context_id)),
            (1, Value::Unsigned(self.surface_id)),
            (2, Value::Unsigned(self.drop_id)),
            (3, Value::Unsigned(self.transfer_id)),
            (4, Value::Unsigned(self.expected_generation.get())),
            (5, Value::Unsigned(self.new_generation.get())),
            (6, Value::Unsigned(self.committed_offset)),
            (7, Value::Unsigned(self.maximum_body_bytes)),
            (8, Value::Unsigned(self.maximum_records)),
        ])
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new(
            "ADVANCE_FILE_TRANSFER",
            payload,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        )?;
        let advance = Self {
            context_id: map.required_u64(0)?,
            surface_id: map.required_u64(1)?,
            drop_id: map.required_u64(2)?,
            transfer_id: map.required_u64(3)?,
            expected_generation: FileTransferGeneration::new(map.required_u64(4)?),
            new_generation: FileTransferGeneration::new(map.required_u64(5)?),
            committed_offset: map.required_u64(6)?,
            maximum_body_bytes: map.required_u64(7)?,
            maximum_records: map.required_u64(8)?,
        };
        advance.validate(header_object_id)?;
        Ok(advance)
    }

    fn validate(self, header_object_id: u64) -> Result<(), MessageError> {
        validate_header_object(header_object_id, self.transfer_id)?;
        for (key, value) in [
            (0, self.context_id),
            (2, self.drop_id),
            (3, self.transfer_id),
        ] {
            require_nonzero("ADVANCE_FILE_TRANSFER", key, value)?;
        }
        self.expected_generation
            .require_nonzero()
            .map_err(|_| invalid_value("ADVANCE_FILE_TRANSFER", 4, "must be nonzero"))?;
        let expected = self
            .expected_generation
            .advance()
            .map_err(|_| invalid_value("ADVANCE_FILE_TRANSFER", 5, "generation exhausted"))?;
        if self.new_generation != expected {
            return Err(invalid_value(
                "ADVANCE_FILE_TRANSFER",
                5,
                "is not exactly the next generation",
            ));
        }
        if (self.maximum_body_bytes == 0) != (self.maximum_records == 0) {
            return Err(invalid_value(
                "ADVANCE_FILE_TRANSFER",
                7,
                "has inconsistent zero initial credit",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTransferAdvanced {
    pub transfer_id: u64,
    pub generation: FileTransferGeneration,
    pub committed_offset: u64,
    pub open_timeout_us: u64,
}

impl FileTransferAdvanced {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        require_nonzero("FILE_TRANSFER_ADVANCED", 0, self.transfer_id)?;
        self.generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_ADVANCED", 1, "must be nonzero"))?;
        validate_deadline("FILE_TRANSFER_ADVANCED", 3, self.open_timeout_us)?;
        Ok(vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.generation.get())),
            (2, Value::Unsigned(self.committed_offset)),
            (3, Value::Unsigned(self.open_timeout_us)),
        ])
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("FILE_TRANSFER_ADVANCED", payload, &[0, 1, 2, 3])?;
        let advanced = Self {
            transfer_id: map.required_u64(0)?,
            generation: FileTransferGeneration::new(map.required_u64(1)?),
            committed_offset: map.required_u64(2)?,
            open_timeout_us: map.required_u64(3)?,
        };
        advanced.payload()?;
        validate_header_object(header_object_id, advanced.transfer_id)?;
        Ok(advanced)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryFileDrop {
    pub drop_id: u64,
}

impl QueryFileDrop {
    pub fn payload(self) -> Result<PayloadMap, MessageError> {
        require_nonzero("QUERY_FILE_DROP", 0, self.drop_id)?;
        Ok(vec![(0, Value::Unsigned(self.drop_id))])
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("QUERY_FILE_DROP", payload, &[0])?;
        let query = Self {
            drop_id: require_nonzero("QUERY_FILE_DROP", 0, map.required_u64(0)?)?,
        };
        validate_header_object(header_object_id, query.drop_id)?;
        Ok(query)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FileDropState {
    Offered = 1,
    Accepted = 2,
    Transferring = 3,
    Committed = 4,
    Cancelled = 5,
    Failed = 6,
}

impl TryFrom<u64> for FileDropState {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Offered),
            2 => Ok(Self::Accepted),
            3 => Ok(Self::Transferring),
            4 => Ok(Self::Committed),
            5 => Ok(Self::Cancelled),
            6 => Ok(Self::Failed),
            _ => Err(invalid_value("FILE_DROP_STATUS", 1, "has an unknown state")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDropStatus {
    pub drop_id: u64,
    pub state: FileDropState,
    pub transfer_id: u64,
    pub generation: FileTransferGeneration,
    pub committed_offset: u64,
    pub result: Option<FileResultCode>,
    pub final_name: String,
}

impl FileDropStatus {
    pub fn payload(&self) -> Result<PayloadMap, MessageError> {
        require_nonzero("FILE_DROP_STATUS", 0, self.drop_id)?;
        if matches!(
            self.state,
            FileDropState::Accepted | FileDropState::Transferring | FileDropState::Committed
        ) {
            require_nonzero("FILE_DROP_STATUS", 2, self.transfer_id)?;
            self.generation
                .require_nonzero()
                .map_err(|_| invalid_value("FILE_DROP_STATUS", 3, "must be nonzero"))?;
        }
        if self.state == FileDropState::Committed {
            if !matches!(
                self.result,
                Some(FileResultCode::Committed | FileResultCode::AlreadyCommitted)
            ) {
                return Err(invalid_value(
                    "FILE_DROP_STATUS",
                    5,
                    "committed state requires a successful result",
                ));
            }
            validate_suggested_name(&self.final_name)?;
        } else if self.state == FileDropState::Cancelled {
            if self.result != Some(FileResultCode::Cancelled) {
                return Err(invalid_value(
                    "FILE_DROP_STATUS",
                    5,
                    "cancelled state requires a cancelled result",
                ));
            }
        } else if self.state == FileDropState::Failed {
            if !matches!(
                self.result,
                Some(
                    FileResultCode::Rejected
                        | FileResultCode::HashMismatch
                        | FileResultCode::IoError
                )
            ) {
                return Err(invalid_value(
                    "FILE_DROP_STATUS",
                    5,
                    "failed state requires a failure result",
                ));
            }
        } else if self.result.is_some() {
            return Err(invalid_value(
                "FILE_DROP_STATUS",
                5,
                "nonterminal state carries a result",
            ));
        }
        if self.state != FileDropState::Committed && !self.final_name.is_empty() {
            return Err(invalid_value(
                "FILE_DROP_STATUS",
                6,
                "has a final name before commit",
            ));
        }
        Ok(vec![
            (0, Value::Unsigned(self.drop_id)),
            (1, Value::Unsigned(self.state as u64)),
            (2, Value::Unsigned(self.transfer_id)),
            (3, Value::Unsigned(self.generation.get())),
            (4, Value::Unsigned(self.committed_offset)),
            (
                5,
                Value::Unsigned(self.result.map_or(u64::MAX, |result| result as u64)),
            ),
            (6, Value::Text(self.final_name.clone())),
        ])
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("FILE_DROP_STATUS", payload, &[0, 1, 2, 3, 4, 5, 6])?;
        let result = match map.required_u64(5)? {
            u64::MAX => None,
            value => Some(value.try_into()?),
        };
        let status = Self {
            drop_id: map.required_u64(0)?,
            state: map.required_u64(1)?.try_into()?,
            transfer_id: map.required_u64(2)?,
            generation: FileTransferGeneration::new(map.required_u64(3)?),
            committed_offset: map.required_u64(4)?,
            result,
            final_name: map.required_text(6)?.to_owned(),
        };
        status.payload()?;
        validate_header_object(header_object_id, status.drop_id)?;
        Ok(status)
    }
}

impl FileDropAccepted {
    pub fn payload(self) -> PayloadMap {
        vec![
            (0, Value::Unsigned(self.drop_id)),
            (1, Value::Unsigned(self.transfer_id)),
            (2, Value::Unsigned(self.transfer_generation.get())),
            (3, Value::Unsigned(self.open_timeout_us)),
        ]
    }

    pub fn decode(header_object_id: u64, payload: &Value) -> Result<Self, MessageError> {
        let map = StrictMap::new("FILE_DROP_ACCEPTED", payload, &[0, 1, 2, 3])?;
        let value = Self {
            drop_id: require_nonzero("FILE_DROP_ACCEPTED", 0, map.required_u64(0)?)?,
            transfer_id: require_nonzero("FILE_DROP_ACCEPTED", 1, map.required_u64(1)?)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(2)?),
            open_timeout_us: map.required_u64(3)?,
        };
        validate_header_object(header_object_id, value.drop_id)?;
        value
            .transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_DROP_ACCEPTED", 2, "must be nonzero"))?;
        validate_deadline("FILE_DROP_ACCEPTED", 3, value.open_timeout_us)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTransferOpen {
    pub session_id: u64,
    pub context_id: u64,
    pub surface_id: u64,
    pub producer_epoch: FileDropEpoch,
    pub grant_generation: FileDropGrantGeneration,
    pub surface_generation: SurfaceGeneration,
    pub drop_id: u64,
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub resume_offset: u64,
    pub maximum_record_body: u32,
    pub maximum_body_bytes: u64,
    pub maximum_records: u64,
    pub client_nonce: [u8; FILE_TRANSFER_NONCE_BYTES],
    pub authentication_tag: [u8; FILE_TRANSFER_TAG_BYTES],
}

impl FileTransferOpen {
    pub fn encode(&self) -> Result<Vec<u8>, MessageError> {
        self.validate()?;
        encode_raw(vec![
            (0, Value::Unsigned(self.session_id)),
            (1, Value::Unsigned(self.context_id)),
            (2, Value::Unsigned(self.surface_id)),
            (3, Value::Unsigned(self.drop_id)),
            (4, Value::Unsigned(self.transfer_id)),
            (5, Value::Unsigned(self.transfer_generation.get())),
            (6, Value::Unsigned(self.resume_offset)),
            (7, Value::Unsigned(u64::from(self.maximum_record_body))),
            (8, Value::Unsigned(self.maximum_body_bytes)),
            (9, Value::Unsigned(self.maximum_records)),
            (10, Value::Bytes(self.client_nonce.to_vec())),
            (11, Value::Bytes(self.authentication_tag.to_vec())),
            (12, Value::Unsigned(self.producer_epoch.get())),
            (13, Value::Unsigned(self.grant_generation.get())),
            (14, Value::Unsigned(self.surface_generation.get())),
        ])
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = zeroize::Zeroizing::new(cbor::decode(body)?);
        let map = StrictMap::new(
            "FILE_TRANSFER_OPEN",
            &value,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        )?;
        let open = Self {
            session_id: map.required_u64(0)?,
            context_id: map.required_u64(1)?,
            surface_id: map.required_u64(2)?,
            producer_epoch: FileDropEpoch::new(map.required_u64(12)?),
            grant_generation: FileDropGrantGeneration::new(map.required_u64(13)?),
            surface_generation: SurfaceGeneration::new(map.required_u64(14)?),
            drop_id: map.required_u64(3)?,
            transfer_id: map.required_u64(4)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(5)?),
            resume_offset: map.required_u64(6)?,
            maximum_record_body: map.required_u32(7)?,
            maximum_body_bytes: map.required_u64(8)?,
            maximum_records: map.required_u64(9)?,
            client_nonce: map.required_fixed_bytes(10)?,
            authentication_tag: map.required_fixed_bytes(11)?,
        };
        open.validate()?;
        Ok(open)
    }

    fn validate(&self) -> Result<(), MessageError> {
        for (key, value) in [
            (0, self.session_id),
            (1, self.context_id),
            (3, self.drop_id),
            (4, self.transfer_id),
        ] {
            require_nonzero("FILE_TRANSFER_OPEN", key, value)?;
        }
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_OPEN", 5, "must be nonzero"))?;
        self.producer_epoch
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_OPEN", 12, "must be nonzero"))?;
        self.grant_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_OPEN", 13, "must be nonzero"))?;
        if (self.surface_id == 0 && self.surface_generation != SurfaceGeneration::ZERO)
            || (self.surface_id != 0 && self.surface_generation.require_nonzero().is_err())
        {
            return Err(invalid_value(
                "FILE_TRANSFER_OPEN",
                14,
                "does not match the scoped surface identity",
            ));
        }
        validate_credit(
            self.maximum_record_body,
            self.maximum_body_bytes,
            self.maximum_records,
            "FILE_TRANSFER_OPEN",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTransferAccepted {
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub resume_offset: u64,
}

impl FileTransferAccepted {
    pub fn encode(self) -> Result<Vec<u8>, MessageError> {
        require_nonzero("FILE_TRANSFER_ACCEPTED", 0, self.transfer_id)?;
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_ACCEPTED", 1, "must be nonzero"))?;
        encode_raw(vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.transfer_generation.get())),
            (2, Value::Unsigned(self.resume_offset)),
        ])
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = cbor::decode(body)?;
        let map = StrictMap::new("FILE_TRANSFER_ACCEPTED", &value, &[0, 1, 2])?;
        let accepted = Self {
            transfer_id: require_nonzero("FILE_TRANSFER_ACCEPTED", 0, map.required_u64(0)?)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(1)?),
            resume_offset: map.required_u64(2)?,
        };
        accepted
            .transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_ACCEPTED", 1, "must be nonzero"))?;
        Ok(accepted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaximumFileData {
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub maximum_body_bytes: u64,
    pub maximum_records: u64,
}

impl MaximumFileData {
    pub fn encode(self) -> Result<Vec<u8>, MessageError> {
        require_nonzero("MAX_FILE_DATA", 0, self.transfer_id)?;
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("MAX_FILE_DATA", 1, "must be nonzero"))?;
        if self.maximum_body_bytes == 0 || self.maximum_records == 0 {
            return Err(invalid_value("MAX_FILE_DATA", 2, "has zero credit"));
        }
        encode_raw(vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.transfer_generation.get())),
            (2, Value::Unsigned(self.maximum_body_bytes)),
            (3, Value::Unsigned(self.maximum_records)),
        ])
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = cbor::decode(body)?;
        let map = StrictMap::new("MAX_FILE_DATA", &value, &[0, 1, 2, 3])?;
        let maximum = Self {
            transfer_id: require_nonzero("MAX_FILE_DATA", 0, map.required_u64(0)?)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(1)?),
            maximum_body_bytes: map.required_u64(2)?,
            maximum_records: map.required_u64(3)?,
        };
        maximum.encode()?;
        Ok(maximum)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFinish {
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub final_length: u64,
    pub sha256: [u8; FILE_HASH_BYTES],
}

impl FileFinish {
    pub fn encode(self) -> Result<Vec<u8>, MessageError> {
        require_nonzero("FILE_FINISH", 0, self.transfer_id)?;
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_FINISH", 1, "must be nonzero"))?;
        encode_raw(vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.transfer_generation.get())),
            (2, Value::Unsigned(self.final_length)),
            (3, Value::Bytes(self.sha256.to_vec())),
        ])
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = cbor::decode(body)?;
        let map = StrictMap::new("FILE_FINISH", &value, &[0, 1, 2, 3])?;
        let finish = Self {
            transfer_id: require_nonzero("FILE_FINISH", 0, map.required_u64(0)?)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(1)?),
            final_length: map.required_u64(2)?,
            sha256: map.required_fixed_bytes(3)?,
        };
        finish
            .transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_FINISH", 1, "must be nonzero"))?;
        Ok(finish)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FileResultCode {
    Committed = 0,
    Rejected = 1,
    Cancelled = 2,
    HashMismatch = 3,
    IoError = 4,
    AlreadyCommitted = 5,
}

impl TryFrom<u64> for FileResultCode {
    type Error = MessageError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Committed),
            1 => Ok(Self::Rejected),
            2 => Ok(Self::Cancelled),
            3 => Ok(Self::HashMismatch),
            4 => Ok(Self::IoError),
            5 => Ok(Self::AlreadyCommitted),
            _ => Err(invalid_value("FILE_RESULT", 2, "has an unknown result")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResult {
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub result: FileResultCode,
    pub committed_length: u64,
    pub final_name: String,
    /// The committed absolute path, carried only under `file-drop-path-v1` on a successful result.
    pub committed_path: Option<String>,
}

impl FileResult {
    pub fn encode(&self) -> Result<Vec<u8>, MessageError> {
        require_nonzero("FILE_RESULT", 0, self.transfer_id)?;
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_RESULT", 1, "must be nonzero"))?;
        if matches!(
            self.result,
            FileResultCode::Committed | FileResultCode::AlreadyCommitted
        ) {
            validate_suggested_name(&self.final_name)?;
        } else if !self.final_name.is_empty() {
            return Err(invalid_value(
                "FILE_RESULT",
                4,
                "must omit the final name on failure",
            ));
        }
        let mut entries = vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.transfer_generation.get())),
            (2, Value::Unsigned(self.result as u64)),
            (3, Value::Unsigned(self.committed_length)),
            (4, Value::Text(self.final_name.clone())),
        ];
        // Absent by default, so a producer without `file-drop-path-v1` re-encodes byte for byte
        // as it did before the profile existed.
        if let Some(path) = &self.committed_path {
            if !matches!(
                self.result,
                FileResultCode::Committed | FileResultCode::AlreadyCommitted
            ) {
                return Err(invalid_value(
                    "FILE_RESULT",
                    5,
                    "is only carried by a committed result",
                ));
            }
            validate_committed_path(path, &self.final_name)?;
            entries.push((5, Value::Text(path.clone())));
        }
        encode_raw(entries)
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = cbor::decode(body)?;
        let map = StrictMap::new("FILE_RESULT", &value, &[0, 1, 2, 3, 4, 5])?;
        let result = Self {
            transfer_id: map.required_u64(0)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(1)?),
            result: map.required_u64(2)?.try_into()?,
            committed_length: map.required_u64(3)?,
            final_name: map.required_text(4)?.to_owned(),
            committed_path: map.optional_text(5)?.map(ToOwned::to_owned),
        };
        result.encode()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTransferAbort {
    pub transfer_id: u64,
    pub transfer_generation: FileTransferGeneration,
    pub reason: u64,
    pub final_offset: u64,
}

impl FileTransferAbort {
    pub fn encode(self) -> Result<Vec<u8>, MessageError> {
        require_nonzero("FILE_TRANSFER_ABORT", 0, self.transfer_id)?;
        self.transfer_generation
            .require_nonzero()
            .map_err(|_| invalid_value("FILE_TRANSFER_ABORT", 1, "must be nonzero"))?;
        if self.reason > 7 {
            return Err(invalid_value(
                "FILE_TRANSFER_ABORT",
                2,
                "has an unknown abort reason",
            ));
        }
        encode_raw(vec![
            (0, Value::Unsigned(self.transfer_id)),
            (1, Value::Unsigned(self.transfer_generation.get())),
            (2, Value::Unsigned(self.reason)),
            (3, Value::Unsigned(self.final_offset)),
        ])
    }

    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let value = cbor::decode(body)?;
        let map = StrictMap::new("FILE_TRANSFER_ABORT", &value, &[0, 1, 2, 3])?;
        let abort = Self {
            transfer_id: map.required_u64(0)?,
            transfer_generation: FileTransferGeneration::new(map.required_u64(1)?),
            reason: map.required_u64(2)?,
            final_offset: map.required_u64(3)?,
        };
        abort.encode()?;
        Ok(abort)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedFileData<'a> {
    pub offset: u64,
    pub data: &'a [u8],
}

pub fn file_data_prefix(
    offset: u64,
    data_length: usize,
) -> io::Result<[u8; FILE_DATA_PREFIX_SIZE]> {
    let data_length = u32::try_from(data_length)
        .ok()
        .filter(|length| *length != 0)
        .ok_or_else(|| invalid_io("file data length is zero or exceeds u32"))?;
    data_length
        .checked_add(FILE_DATA_PREFIX_SIZE as u32)
        .filter(|length| *length <= HARD_MAX_RECORD_BODY)
        .ok_or_else(|| invalid_io("file data body exceeds the hard record limit"))?;
    let mut prefix = [0_u8; FILE_DATA_PREFIX_SIZE];
    prefix[..8].copy_from_slice(&offset.to_be_bytes());
    prefix[8..12].copy_from_slice(&data_length.to_be_bytes());
    Ok(prefix)
}

pub fn parse_file_data(body: &[u8]) -> io::Result<ParsedFileData<'_>> {
    if body.len() < FILE_DATA_PREFIX_SIZE {
        return Err(invalid_io("file data is shorter than its prefix"));
    }
    let offset = u64::from_be_bytes(body[..8].try_into().expect("checked prefix"));
    let data_length = u32::from_be_bytes(body[8..12].try_into().expect("checked prefix")) as usize;
    if body[12..16] != [0; 4]
        || data_length == 0
        || data_length != body.len() - FILE_DATA_PREFIX_SIZE
    {
        return Err(invalid_io(
            "file data has invalid length or reserved fields",
        ));
    }
    Ok(ParsedFileData {
        offset,
        data: &body[FILE_DATA_PREFIX_SIZE..],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTransferFlow {
    generation: FileTransferGeneration,
    next_offset: u64,
    sent_body_bytes: u64,
    sent_records: u64,
    maximum_body_bytes: u64,
    maximum_records: u64,
}

impl FileTransferFlow {
    pub fn new(
        generation: FileTransferGeneration,
        resume_offset: u64,
        maximum_body_bytes: u64,
        maximum_records: u64,
    ) -> Result<Self, MessageError> {
        generation
            .require_nonzero()
            .map_err(|_| invalid_value("file-transfer flow", 0, "has a zero generation"))?;
        if (maximum_body_bytes == 0) != (maximum_records == 0) {
            return Err(invalid_value(
                "file-transfer flow",
                1,
                "has inconsistent zero credit",
            ));
        }
        Ok(Self {
            generation,
            next_offset: resume_offset,
            sent_body_bytes: 0,
            sent_records: 0,
            maximum_body_bytes,
            maximum_records,
        })
    }

    pub const fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub fn admit(&mut self, offset: u64, payload_length: u32) -> Result<(), MessageError> {
        if offset != self.next_offset || payload_length == 0 {
            return Err(invalid_value(
                "FILE_DATA",
                0,
                "has a nonsequential offset or empty payload",
            ));
        }
        let body_length = u64::from(payload_length)
            .checked_add(FILE_DATA_PREFIX_SIZE as u64)
            .ok_or_else(|| invalid_value("FILE_DATA", 1, "body length overflows"))?;
        let body_bytes = self
            .sent_body_bytes
            .checked_add(body_length)
            .ok_or_else(|| invalid_value("FILE_DATA", 1, "byte counter overflows"))?;
        let records = self
            .sent_records
            .checked_add(1)
            .ok_or_else(|| invalid_value("FILE_DATA", 1, "record counter overflows"))?;
        if body_bytes > self.maximum_body_bytes || records > self.maximum_records {
            return Err(invalid_value("FILE_DATA", 1, "exceeds cumulative credit"));
        }
        self.next_offset = self
            .next_offset
            .checked_add(u64::from(payload_length))
            .ok_or_else(|| invalid_value("FILE_DATA", 0, "file offset overflows"))?;
        self.sent_body_bytes = body_bytes;
        self.sent_records = records;
        Ok(())
    }

    pub fn raise_maxima(&mut self, body_bytes: u64, records: u64) {
        self.maximum_body_bytes = self.maximum_body_bytes.max(body_bytes);
        self.maximum_records = self.maximum_records.max(records);
    }

    pub fn advance(
        &mut self,
        generation: FileTransferGeneration,
        committed_offset: u64,
        maximum_body_bytes: u64,
        maximum_records: u64,
    ) -> Result<(), MessageError> {
        if generation
            != self
                .generation
                .advance()
                .map_err(|_| invalid_value("ADVANCE_FILE_TRANSFER", 0, "generation exhausted"))?
            || committed_offset > self.next_offset
        {
            return Err(invalid_value(
                "ADVANCE_FILE_TRANSFER",
                0,
                "has a nonconsecutive generation or impossible offset",
            ));
        }
        *self = Self::new(
            generation,
            committed_offset,
            maximum_body_bytes,
            maximum_records,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDropBindingOutcome {
    Disabled,
    Enabled(FileDropGrant),
    Denied(FileDropGrant),
    ExactRetry(FileDropGrant),
}

#[derive(Debug, Default, Clone)]
pub struct FileDropGate {
    latest: Option<FileDropBinding>,
    generation: FileDropGrantGeneration,
    current: Option<FileDropGrant>,
    last_result: Option<FileDropGrant>,
}

impl FileDropGate {
    pub fn current(&self) -> Option<FileDropGrant> {
        self.current
    }

    pub fn apply(
        &mut self,
        binding: FileDropBinding,
        allow: bool,
    ) -> Result<FileDropBindingOutcome, MessageError> {
        binding.validate(binding.surface_id)?;
        if let Some(previous) = &self.latest {
            if binding.producer_epoch < previous.producer_epoch {
                return Err(invalid_value(
                    "SET_FILE_DROP_BINDING",
                    0,
                    "moves the epoch backward",
                ));
            }
            if binding.producer_epoch == previous.producer_epoch {
                return if binding == *previous {
                    Ok(FileDropBindingOutcome::ExactRetry(
                        self.last_result.expect("a previous binding has a result"),
                    ))
                } else {
                    Err(invalid_value(
                        "SET_FILE_DROP_BINDING",
                        0,
                        "reuses an epoch with different bytes",
                    ))
                };
            }
        }
        if binding.disabled() {
            let result = FileDropGrant {
                producer_epoch: binding.producer_epoch,
                grant_generation: FileDropGrantGeneration::ZERO,
                context_id: binding.context_id,
                surface_id: binding.surface_id,
                surface_generation: binding.surface_generation,
                state: FileDropBindingState::Disabled,
                destination: None,
                maximum_file_bytes: 0,
                maximum_pending_offers: 0,
                maximum_active_transfers: 0,
                maximum_record_body: 0,
                acceptance_timeout_us: 0,
                idle_timeout_us: 0,
                reason: 0,
            };
            self.latest = Some(binding);
            self.current = None;
            self.last_result = Some(result);
            return Ok(FileDropBindingOutcome::Disabled);
        }
        let generation = self
            .generation
            .advance()
            .map_err(|_| invalid_value("FILE_DROP_BOUND", 1, "grant generation exhausted"))?;
        let state = if allow {
            FileDropBindingState::Enabled
        } else {
            FileDropBindingState::Denied
        };
        let grant = FileDropGrant {
            producer_epoch: binding.producer_epoch,
            grant_generation: generation,
            context_id: binding.context_id,
            surface_id: binding.surface_id,
            surface_generation: binding.surface_generation,
            state,
            destination: binding.destination,
            maximum_file_bytes: binding.maximum_file_bytes,
            maximum_pending_offers: binding.maximum_pending_offers,
            maximum_active_transfers: binding.maximum_active_transfers,
            maximum_record_body: binding.maximum_record_body,
            acceptance_timeout_us: binding.acceptance_timeout_us,
            idle_timeout_us: binding.idle_timeout_us,
            reason: 0,
        };
        self.latest = Some(binding);
        self.generation = generation;
        self.current = allow.then_some(grant);
        self.last_result = Some(grant);
        Ok(if allow {
            FileDropBindingOutcome::Enabled(grant)
        } else {
            FileDropBindingOutcome::Denied(grant)
        })
    }
}

fn validate_binding_limits(
    pending: u64,
    active: u64,
    record_body: u32,
    acceptance_timeout_us: u64,
    idle_timeout_us: u64,
    schema: &'static str,
) -> Result<(), MessageError> {
    if pending == 0
        || pending > MAX_PENDING_FILE_DROPS
        || active == 0
        || active > MAX_ACTIVE_FILE_TRANSFERS
        || active > pending
    {
        return Err(invalid_value(
            schema,
            6,
            "has invalid pending or active limits",
        ));
    }
    if record_body <= FILE_DATA_PREFIX_SIZE as u32 || record_body > HARD_MAX_RECORD_BODY {
        return Err(invalid_value(schema, 8, "has an invalid record-body limit"));
    }
    validate_deadline(schema, 9, acceptance_timeout_us)?;
    validate_deadline(schema, 10, idle_timeout_us)
}

fn validate_credit(
    record_body: u32,
    maximum_body_bytes: u64,
    maximum_records: u64,
    schema: &'static str,
) -> Result<(), MessageError> {
    if record_body <= FILE_DATA_PREFIX_SIZE as u32 || record_body > HARD_MAX_RECORD_BODY {
        return Err(invalid_value(schema, 8, "has an invalid record-body limit"));
    }
    if maximum_body_bytes == 0 && maximum_records == 0 {
        return Ok(());
    }
    if maximum_records == 0 || maximum_body_bytes < u64::from(record_body) {
        return Err(invalid_value(
            schema,
            9,
            "does not grant one maximum legal record",
        ));
    }
    Ok(())
}

fn validate_deadline(schema: &'static str, key: u64, timeout_us: u64) -> Result<(), MessageError> {
    if !(MIN_FILE_DROP_DEADLINE_US..=MAX_FILE_DROP_DEADLINE_US).contains(&timeout_us) {
        Err(invalid_value(
            schema,
            key,
            "is outside the finite timeout range",
        ))
    } else {
        Ok(())
    }
}

/// Validate a producer-supplied absolute destination path from `FILE_RESULT` key 5.
///
/// This is the one place `file-drop-v1` discloses a path, and a presenter may type it into a
/// terminal, so the value is rejected outright rather than repaired. `char::is_control` covers
/// `\n`, `\r`, `\x1b`, `\x03`, `\x07`, and NUL, all of which a Linux directory name may
/// legally contain. Pinning the final component to the already-validated `final_name` leaves the
/// directory prefix as the only producer-controlled part, and that is bounded to absolute,
/// control-free, non-`..` components.
pub fn validate_committed_path(path: &str, final_name: &str) -> Result<(), MessageError> {
    if path.is_empty()
        || path.len() > MAX_COMMITTED_PATH_BYTES
        || !path.starts_with('/')
        || path.chars().any(char::is_control)
        || path.split('/').any(|component| component == "..")
        || path.rsplit('/').next() != Some(final_name)
    {
        return Err(invalid_value(
            "FILE_RESULT",
            5,
            "is not a safe absolute committed path",
        ));
    }
    Ok(())
}

pub fn validate_suggested_name(name: &str) -> Result<(), MessageError> {
    if name.is_empty()
        || name.len() > MAX_FILE_DROP_NAME_BYTES
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|character| {
            matches!(
                character,
                '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
            )
        })
        || name.chars().any(char::is_control)
        || is_windows_reserved_name(name)
    {
        return Err(invalid_value(
            "file-drop name",
            6,
            "is empty, unsafe, reserved, or too long",
        ));
    }
    Ok(())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn encode_raw(payload: PayloadMap) -> Result<Vec<u8>, MessageError> {
    Ok(cbor::encode(&zeroize::Zeroizing::new(Value::Map(payload)))?)
}

fn invalid_io(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(epoch: u64) -> FileDropBinding {
        FileDropBinding {
            producer_epoch: FileDropEpoch::new(epoch),
            context_id: 2,
            surface_id: 0,
            surface_generation: SurfaceGeneration::ZERO,
            destination: Some(FileDropDestination::ShellCwd),
            maximum_file_bytes: 1_000_000,
            maximum_pending_offers: DEFAULT_PENDING_FILE_DROPS,
            maximum_active_transfers: DEFAULT_ACTIVE_FILE_TRANSFERS,
            maximum_record_body: 65_536,
            acceptance_timeout_us: DEFAULT_FILE_DROP_ACCEPTANCE_US,
            idle_timeout_us: DEFAULT_FILE_TRANSFER_IDLE_US,
        }
    }

    #[test]
    fn exhausted_binding_preserves_result_and_other_owner() {
        let mut gate = FileDropGate::default();
        gate.apply(binding(1), true).unwrap();
        // Seed the reachable terminal counter without iterating through u64::MAX grants.
        gate.generation = FileDropGrantGeneration::new(u64::MAX);
        let before = gate.clone();
        let mut other = FileDropGate::default();
        let mut other_binding = binding(1);
        other_binding.context_id = 9;
        other.apply(other_binding, true).unwrap();
        let other_before = other.current();
        for allow in [true, false] {
            for _ in 0..2 {
                assert!(gate.apply(binding(2), allow).is_err());
                assert_eq!(gate.latest, before.latest);
                assert_eq!(gate.generation, before.generation);
                assert_eq!(gate.current, before.current);
                assert_eq!(gate.last_result, before.last_result);
                assert_eq!(other.current(), other_before);
            }
        }
        assert_eq!(
            gate.apply(binding(1), true).unwrap(),
            FileDropBindingOutcome::ExactRetry(before.last_result.unwrap())
        );
    }

    #[test]
    fn binding_and_offer_round_trip_strictly() {
        let binding = binding(1);
        let decoded = FileDropBinding::decode(0, &Value::Map(binding.payload().unwrap())).unwrap();
        assert_eq!(decoded, binding);

        let offer = FileDropOffer {
            binding: FileDropTuple {
                producer_epoch: FileDropEpoch::ONE,
                grant_generation: FileDropGrantGeneration::ONE,
                context_id: 2,
                surface_id: 0,
                surface_generation: SurfaceGeneration::ZERO,
                drop_id: 7,
            },
            suggested_name: "report.pdf".into(),
            declared_length: 99,
        };
        assert_eq!(
            FileDropOffer::decode(7, &Value::Map(offer.payload().unwrap())).unwrap(),
            offer
        );
    }

    #[test]
    fn disable_is_scoped_to_the_binding_identity() {
        let mut disabled = binding(2);
        disabled.surface_id = 9;
        disabled.surface_generation = SurfaceGeneration::new(4);
        disabled.destination = None;
        disabled.maximum_file_bytes = 0;
        disabled.maximum_pending_offers = 0;
        disabled.maximum_active_transfers = 0;
        disabled.maximum_record_body = 0;
        disabled.acceptance_timeout_us = 0;
        disabled.idle_timeout_us = 0;
        let decoded = FileDropBinding::decode(
            disabled.surface_id,
            &Value::Map(disabled.payload().unwrap()),
        )
        .unwrap();
        assert_eq!(decoded.context_id, 2);
        assert_eq!(decoded.surface_id, 9);
        assert_eq!(decoded.surface_generation, SurfaceGeneration::new(4));

        let mut gate = FileDropGate::default();
        let mut enabled = disabled.clone();
        enabled.producer_epoch = FileDropEpoch::ONE;
        enabled.destination = Some(FileDropDestination::DesktopFolder);
        enabled.maximum_file_bytes = 1;
        enabled.maximum_pending_offers = 1;
        enabled.maximum_active_transfers = 1;
        enabled.maximum_record_body = 1024;
        enabled.acceptance_timeout_us = MIN_FILE_DROP_DEADLINE_US;
        enabled.idle_timeout_us = MIN_FILE_DROP_DEADLINE_US;
        gate.apply(enabled, true).unwrap();
        assert_eq!(
            gate.apply(disabled, true).unwrap(),
            FileDropBindingOutcome::Disabled
        );
        let replay = gate.apply(gate.latest.clone().unwrap(), true).unwrap();
        let FileDropBindingOutcome::ExactRetry(reply) = replay else {
            panic!("disable retry did not replay its named reply")
        };
        assert_eq!(reply.grant_generation, FileDropGrantGeneration::ZERO);
        assert_eq!((reply.context_id, reply.surface_id), (2, 9));
    }

    #[test]
    fn transfer_open_and_finish_round_trip() {
        let open = FileTransferOpen {
            session_id: 1,
            context_id: 2,
            surface_id: 0,
            producer_epoch: FileDropEpoch::ONE,
            grant_generation: FileDropGrantGeneration::ONE,
            surface_generation: SurfaceGeneration::ZERO,
            drop_id: 3,
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            resume_offset: 0,
            maximum_record_body: 1024,
            maximum_body_bytes: 1024,
            maximum_records: 1,
            client_nonce: [5; 16],
            authentication_tag: [6; 16],
        };
        assert_eq!(
            FileTransferOpen::decode(&open.encode().unwrap()).unwrap(),
            open
        );

        let finish = FileFinish {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            final_length: 9,
            sha256: [7; 32],
        };
        assert_eq!(
            FileFinish::decode(&finish.encode().unwrap()).unwrap(),
            finish
        );
    }

    #[test]
    fn every_file_drop_record_round_trips_and_rejects_unknown_keys() {
        let tuple = FileDropTuple {
            producer_epoch: FileDropEpoch::ONE,
            grant_generation: FileDropGrantGeneration::ONE,
            context_id: 2,
            surface_id: 0,
            surface_generation: SurfaceGeneration::ZERO,
            drop_id: 3,
        };
        let acceptance = AcceptFileDrop {
            binding: tuple,
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            maximum_record_body: 1024,
            initial_maximum_body_bytes: 0,
            initial_maximum_records: 0,
        };
        assert_eq!(
            AcceptFileDrop::decode(3, &Value::Map(acceptance.payload().unwrap())).unwrap(),
            acceptance
        );
        let accepted = FileDropAccepted {
            drop_id: 3,
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            open_timeout_us: MIN_FILE_DROP_DEADLINE_US,
        };
        assert_eq!(
            FileDropAccepted::decode(3, &Value::Map(accepted.payload())).unwrap(),
            accepted
        );
        let cancellation = CancelFileDrop {
            binding: tuple,
            reason: 1,
        };
        assert_eq!(
            CancelFileDrop::decode(
                "FILE_DROP_CANCELLED",
                3,
                &Value::Map(cancellation.payload().unwrap()),
            )
            .unwrap(),
            cancellation
        );
        let advance = AdvanceFileTransfer {
            context_id: 2,
            surface_id: 0,
            drop_id: 3,
            transfer_id: 4,
            expected_generation: FileTransferGeneration::ONE,
            new_generation: FileTransferGeneration::new(2),
            committed_offset: 9,
            maximum_body_bytes: 0,
            maximum_records: 0,
        };
        assert_eq!(
            AdvanceFileTransfer::decode(4, &Value::Map(advance.payload().unwrap())).unwrap(),
            advance
        );
        let advanced = FileTransferAdvanced {
            transfer_id: 4,
            generation: FileTransferGeneration::new(2),
            committed_offset: 9,
            open_timeout_us: MIN_FILE_DROP_DEADLINE_US,
        };
        assert_eq!(
            FileTransferAdvanced::decode(4, &Value::Map(advanced.payload().unwrap())).unwrap(),
            advanced
        );
        let query = QueryFileDrop { drop_id: 3 };
        assert_eq!(
            QueryFileDrop::decode(3, &Value::Map(query.payload().unwrap())).unwrap(),
            query
        );
        let status = FileDropStatus {
            drop_id: 3,
            state: FileDropState::Cancelled,
            transfer_id: 4,
            generation: FileTransferGeneration::ONE,
            committed_offset: 9,
            result: Some(FileResultCode::Cancelled),
            final_name: String::new(),
        };
        assert_eq!(
            FileDropStatus::decode(3, &Value::Map(status.payload().unwrap())).unwrap(),
            status
        );

        let transfer_accepted = FileTransferAccepted {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            resume_offset: 9,
        };
        assert_eq!(
            FileTransferAccepted::decode(&transfer_accepted.encode().unwrap()).unwrap(),
            transfer_accepted
        );
        let maximum = MaximumFileData {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            maximum_body_bytes: 4096,
            maximum_records: 4,
        };
        assert_eq!(
            MaximumFileData::decode(&maximum.encode().unwrap()).unwrap(),
            maximum
        );
        let finish = FileFinish {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            final_length: 9,
            sha256: [7; 32],
        };
        assert_eq!(
            FileFinish::decode(&finish.encode().unwrap()).unwrap(),
            finish
        );
        let result = FileResult {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            result: FileResultCode::Committed,
            committed_length: 9,
            final_name: "file.txt".into(),
            committed_path: None,
        };
        assert_eq!(
            FileResult::decode(&result.encode().unwrap()).unwrap(),
            result
        );
        let abort = FileTransferAbort {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            reason: 2,
            final_offset: 9,
        };
        assert_eq!(
            FileTransferAbort::decode(&abort.encode().unwrap()).unwrap(),
            abort
        );

        let mut unknown = binding(1).payload().unwrap();
        unknown.push((99, Value::Unsigned(1)));
        assert!(FileDropBinding::decode(0, &Value::Map(unknown)).is_err());
        assert!(FileTransferAccepted::decode(&[0xa1, 0x18, 0x00, 0x01]).is_err());
    }

    #[test]
    fn data_prefix_and_flow_reject_gaps_and_over_credit() {
        let prefix = file_data_prefix(10, 3).unwrap();
        let mut body = prefix.to_vec();
        body.extend_from_slice(b"abc");
        let parsed = parse_file_data(&body).unwrap();
        assert_eq!(parsed.offset, 10);
        assert_eq!(parsed.data, b"abc");

        let mut flow = FileTransferFlow::new(FileTransferGeneration::ONE, 10, 19, 1).unwrap();
        flow.admit(10, 3).unwrap();
        assert_eq!(flow.next_offset(), 13);
        assert!(flow.admit(12, 1).is_err());
        assert!(flow.admit(13, 1).is_err());

        let mut stopped = FileTransferFlow::new(FileTransferGeneration::ONE, 0, 0, 0).unwrap();
        assert!(stopped.admit(0, 1).is_err());
        stopped.raise_maxima(17, 1);
        stopped.admit(0, 1).unwrap();
    }

    #[test]
    fn gate_rejects_epoch_reuse_and_keeps_owner_state_separate() {
        let mut first = FileDropGate::default();
        let mut second = FileDropGate::default();
        assert!(matches!(
            first.apply(binding(1), true).unwrap(),
            FileDropBindingOutcome::Enabled(_)
        ));
        assert!(matches!(
            second.apply(binding(1), true).unwrap(),
            FileDropBindingOutcome::Enabled(_)
        ));
        let mut changed = binding(1);
        changed.maximum_file_bytes += 1;
        assert!(first.apply(changed, true).is_err());
        assert!(second.current().is_some());
    }

    #[test]
    fn unsafe_names_are_rejected() {
        for name in [
            "",
            ".",
            "..",
            "../secret",
            "a/b",
            "a\\b",
            "NUL",
            "COM¹.txt",
            "stream:name",
            "trailing.",
        ] {
            assert!(validate_suggested_name(name).is_err(), "accepted {name:?}");
        }
        assert!(validate_suggested_name("-option").is_ok());
        assert!(validate_suggested_name("résumé.txt").is_ok());
    }

    #[test]
    fn bounded_codec_fuzz_smoke_never_panics() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for length in 0..=255_usize {
            for _ in 0..16 {
                let mut bytes = vec![0_u8; length];
                for byte in &mut bytes {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    *byte = state as u8;
                }
                let _ = FileTransferOpen::decode(&bytes);
                let _ = FileTransferAccepted::decode(&bytes);
                let _ = MaximumFileData::decode(&bytes);
                let _ = FileFinish::decode(&bytes);
                let _ = FileResult::decode(&bytes);
                let _ = FileTransferAbort::decode(&bytes);
                let _ = parse_file_data(&bytes);

                if let Ok(value) = cbor::decode(&bytes) {
                    let _ = FileDropBinding::decode(0, &value);
                    let _ = FileDropOffer::decode(1, &value);
                    let _ = AcceptFileDrop::decode(1, &value);
                    let _ = FileDropAccepted::decode(1, &value);
                    let _ = CancelFileDrop::decode("CANCEL_FILE_DROP", 1, &value);
                    let _ = AdvanceFileTransfer::decode(1, &value);
                    let _ = FileTransferAdvanced::decode(1, &value);
                    let _ = QueryFileDrop::decode(1, &value);
                    let _ = FileDropStatus::decode(1, &value);
                }
            }
        }
    }

    fn committed(path: Option<&str>, name: &str, code: FileResultCode) -> FileResult {
        FileResult {
            transfer_id: 4,
            transfer_generation: FileTransferGeneration::ONE,
            result: code,
            committed_length: 9,
            final_name: name.into(),
            committed_path: path.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn a_committed_path_round_trips() {
        for (path, name) in [
            ("/home/u/report.txt", "report.txt"),
            ("/report.txt", "report.txt"),
            ("/home/u/my report.txt", "my report.txt"),
            ("/home/u (1)/report (1).txt", "report (1).txt"),
        ] {
            let result = committed(Some(path), name, FileResultCode::Committed);
            assert_eq!(
                FileResult::decode(&result.encode().unwrap()).unwrap(),
                result
            );
        }
        let replay = committed(
            Some("/home/u/report.txt"),
            "report.txt",
            FileResultCode::AlreadyCommitted,
        );
        assert_eq!(
            FileResult::decode(&replay.encode().unwrap()).unwrap(),
            replay
        );
    }

    #[test]
    fn an_absent_committed_path_encodes_exactly_as_it_did_before_the_profile() {
        // The pre-`file-drop-path-v1` bytes, pinned so a producer that never negotiated the
        // profile can never be told apart from an older one.
        let result = committed(None, "file.txt", FileResultCode::Committed);
        assert_eq!(
            result.encode().unwrap(),
            vec![
                0xa5, 0x00, 0x04, 0x01, 0x01, 0x02, 0x00, 0x03, 0x09, 0x04, 0x68, 0x66, 0x69, 0x6c,
                0x65, 0x2e, 0x74, 0x78, 0x74,
            ]
        );
    }

    #[test]
    fn a_committed_path_is_refused_on_every_failure_result() {
        for code in [
            FileResultCode::Rejected,
            FileResultCode::Cancelled,
            FileResultCode::HashMismatch,
            FileResultCode::IoError,
        ] {
            // A failure result also carries no final name, so both guards must hold.
            assert!(
                committed(Some("/home/u/report.txt"), "", code)
                    .encode()
                    .is_err()
            );
        }
    }

    #[test]
    fn an_unsafe_committed_path_is_refused_rather_than_repaired() {
        let unsafe_paths = [
            "home/u/report.txt",
            "",
            "/home/u/../etc/report.txt",
            "/home/u/other.txt",
            "/home/u/report.txt/",
            "/home/u/",
        ];
        for path in unsafe_paths {
            assert!(
                committed(Some(path), "report.txt", FileResultCode::Committed)
                    .encode()
                    .is_err(),
                "{path:?} was accepted"
            );
        }
        // A Linux directory name may legally hold any of these, so none may reach a terminal.
        for control in ['\n', '\r', '\u{1b}', '\u{3}', '\u{7}', '\0'] {
            let path = format!("/home/u{control}x/report.txt");
            assert!(
                committed(Some(&path), "report.txt", FileResultCode::Committed)
                    .encode()
                    .is_err(),
                "{control:?} was accepted"
            );
        }
        let long = format!("/{}/report.txt", "d".repeat(MAX_COMMITTED_PATH_BYTES));
        assert!(
            committed(Some(&long), "report.txt", FileResultCode::Committed)
                .encode()
                .is_err()
        );
    }

    #[test]
    fn decoding_enforces_every_committed_path_rule() {
        let good = committed(
            Some("/home/u/report.txt"),
            "report.txt",
            FileResultCode::Committed,
        );
        let body = good.encode().unwrap();
        assert!(FileResult::decode(&body).is_ok());

        // Hand-rolled records that `encode` would never produce still have to be rejected.
        let relative = encode_raw(vec![
            (0, Value::Unsigned(4)),
            (1, Value::Unsigned(1)),
            (2, Value::Unsigned(FileResultCode::Committed as u64)),
            (3, Value::Unsigned(9)),
            (4, Value::Text("report.txt".into())),
            (5, Value::Text("home/u/report.txt".into())),
        ])
        .unwrap();
        assert!(FileResult::decode(&relative).is_err());

        let wrong_type = encode_raw(vec![
            (0, Value::Unsigned(4)),
            (1, Value::Unsigned(1)),
            (2, Value::Unsigned(FileResultCode::Committed as u64)),
            (3, Value::Unsigned(9)),
            (4, Value::Text("report.txt".into())),
            (5, Value::Bytes(b"/home/u/report.txt".to_vec())),
        ])
        .unwrap();
        assert!(FileResult::decode(&wrong_type).is_err());

        let unknown_key = encode_raw(vec![
            (0, Value::Unsigned(4)),
            (1, Value::Unsigned(1)),
            (2, Value::Unsigned(FileResultCode::Committed as u64)),
            (3, Value::Unsigned(9)),
            (4, Value::Text("report.txt".into())),
            (6, Value::Text("/home/u/report.txt".into())),
        ])
        .unwrap();
        assert!(FileResult::decode(&unknown_key).is_err());
    }
}
