//! Native Vivid 1.5 endpoint and root-secret discovery.

use std::{env, fmt, io};

use crate::{
    auth::{AuthError, Secret32},
    messages::LaneClass,
    wire::Endpoint,
};

pub const ENDPOINT_CONTROL: &str = "VIVID_ENDPOINT_CONTROL";
pub const ENDPOINT_INTERACTIVE: &str = "VIVID_ENDPOINT_INTERACTIVE";
pub const ENDPOINT_REALTIME: &str = "VIVID_ENDPOINT_REALTIME";
pub const ENDPOINT_BULK: &str = "VIVID_ENDPOINT_BULK";
pub const ROOT_SECRET: &str = "VIVID_ROOT_SECRET";

pub struct NativeDiscovery {
    control: Endpoint,
    interactive: Option<Endpoint>,
    realtime: Option<Endpoint>,
    bulk: Option<Endpoint>,
    root_secret: Secret32,
}

impl fmt::Debug for NativeDiscovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeDiscovery")
            .field("control", &"[REDACTED ENDPOINT]")
            .field(
                "interactive",
                &self.interactive.as_ref().map(|_| "[REDACTED ENDPOINT]"),
            )
            .field(
                "realtime",
                &self.realtime.as_ref().map(|_| "[REDACTED ENDPOINT]"),
            )
            .field("bulk", &self.bulk.as_ref().map(|_| "[REDACTED ENDPOINT]"))
            .field("root_secret", &"[REDACTED]")
            .finish()
    }
}

impl NativeDiscovery {
    pub fn from_environment() -> io::Result<Self> {
        let control = required_endpoint(ENDPOINT_CONTROL)?;
        let interactive = optional_endpoint(ENDPOINT_INTERACTIVE)?;
        let realtime = optional_endpoint(ENDPOINT_REALTIME)?;
        let bulk = optional_endpoint(ENDPOINT_BULK)?;
        let root_secret = env::var(ROOT_SECRET)
            .map_err(|_| missing(ROOT_SECRET))
            .and_then(|value| {
                Secret32::from_hex(&value).map_err(|error| secret_error(ROOT_SECRET, error))
            })?;
        Ok(Self {
            control,
            interactive,
            realtime,
            bulk,
            root_secret,
        })
    }

    pub fn control(&self) -> &Endpoint {
        &self.control
    }

    pub fn interactive(&self) -> &Endpoint {
        self.interactive.as_ref().unwrap_or(&self.control)
    }

    pub fn track(&self, lane: LaneClass) -> io::Result<&Endpoint> {
        match lane {
            LaneClass::Realtime => Ok(self
                .realtime
                .as_ref()
                .or(self.bulk.as_ref())
                .unwrap_or(&self.control)),
            LaneClass::Bulk => Ok(self.bulk.as_ref().unwrap_or(&self.control)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "track endpoint requires realtime or bulk lane",
            )),
        }
    }

    pub fn root_secret(&self) -> &Secret32 {
        &self.root_secret
    }
}

fn required_endpoint(name: &'static str) -> io::Result<Endpoint> {
    env::var(name)
        .map_err(|_| missing(name))
        .and_then(|value| Endpoint::parse(&value))
}

fn optional_endpoint(name: &'static str) -> io::Result<Option<Endpoint>> {
    env::var(name)
        .ok()
        .map(|value| Endpoint::parse(&value))
        .transpose()
}

fn missing(name: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("required Vivid discovery variable {name} is absent"),
    )
}

fn secret_error(name: &'static str, error: AuthError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{name} has invalid encoding: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_discovery_values() {
        let discovery = NativeDiscovery {
            control: Endpoint::Tcp("127.0.0.1:1234".into()),
            interactive: None,
            realtime: None,
            bulk: None,
            root_secret: Secret32::new([7; 32]),
        };
        let debug = format!("{discovery:?}");
        assert!(!debug.contains("1234"));
        assert!(!debug.contains("0707"));
    }

    #[test]
    fn fallback_selects_values_without_inventing_endpoint_retries() {
        let discovery = NativeDiscovery {
            control: Endpoint::Tcp("127.0.0.1:1234".into()),
            interactive: None,
            realtime: None,
            bulk: Some(Endpoint::Tcp("127.0.0.1:5678".into())),
            root_secret: Secret32::new([7; 32]),
        };
        assert_eq!(discovery.interactive(), discovery.control());
        assert_eq!(
            discovery.track(LaneClass::Realtime).unwrap(),
            discovery.track(LaneClass::Bulk).unwrap()
        );
    }
}
