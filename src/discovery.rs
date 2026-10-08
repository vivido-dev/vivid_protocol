//! Native Vivid 1.5 endpoint and root-secret discovery.

use std::{env, fmt, io};
use zeroize::Zeroizing;

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
    /// Reads discovery from the process environment.
    ///
    /// A variable that is absent or not valid Unicode counts as unset.
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_lookup`].
    pub fn from_environment() -> io::Result<Self> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// Reads discovery through `lookup`, which maps a variable name to its value.
    ///
    /// This is the environment-free form of [`Self::from_environment`], for embedders that
    /// receive discovery some other way and for tests.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when the control endpoint or root secret is unset, `InvalidInput` for a
    /// malformed endpoint, and `InvalidData` for a root secret that is not 64 hex digits.
    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
        let control = required_endpoint(&mut lookup, ENDPOINT_CONTROL)?;
        let interactive = optional_endpoint(&mut lookup, ENDPOINT_INTERACTIVE)?;
        let realtime = optional_endpoint(&mut lookup, ENDPOINT_REALTIME)?;
        let bulk = optional_endpoint(&mut lookup, ENDPOINT_BULK)?;
        let value = Zeroizing::new(lookup(ROOT_SECRET).ok_or_else(|| missing(ROOT_SECRET))?);
        let root_secret =
            Secret32::from_hex(&value).map_err(|error| secret_error(ROOT_SECRET, error))?;
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

fn required_endpoint(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> io::Result<Endpoint> {
    lookup(name)
        .ok_or_else(|| missing(name))
        .and_then(|value| Endpoint::parse(&value))
}

fn optional_endpoint(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> io::Result<Option<Endpoint>> {
    lookup(name)
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

    fn lookup<'a>(vars: &'a [(&str, &str)]) -> impl FnMut(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn lookup_reads_discovery_without_the_process_environment() {
        let secret = "0123456789abcdef".repeat(4);
        let discovery = NativeDiscovery::from_lookup(lookup(&[
            (ENDPOINT_CONTROL, "tcp:127.0.0.1:1234"),
            (ENDPOINT_BULK, "tcp:127.0.0.1:5678"),
            (ROOT_SECRET, &secret),
        ]))
        .unwrap();
        assert_eq!(discovery.control(), &Endpoint::Tcp("127.0.0.1:1234".into()));
        assert_eq!(discovery.interactive(), discovery.control());
        assert_eq!(
            discovery.track(LaneClass::Bulk).unwrap(),
            &Endpoint::Tcp("127.0.0.1:5678".into())
        );
        assert_eq!(
            discovery.root_secret().expose(),
            Secret32::from_hex(&secret).unwrap().expose()
        );
    }

    #[test]
    fn lookup_rejects_missing_or_malformed_values_without_echoing_secrets() {
        let missing_control = NativeDiscovery::from_lookup(lookup(&[(ROOT_SECRET, "00")]));
        assert_eq!(missing_control.unwrap_err().kind(), io::ErrorKind::NotFound);

        let missing_secret =
            NativeDiscovery::from_lookup(lookup(&[(ENDPOINT_CONTROL, "tcp:127.0.0.1:1234")]));
        assert_eq!(missing_secret.unwrap_err().kind(), io::ErrorKind::NotFound);

        let bad_secret = "zz".repeat(32);
        let error = NativeDiscovery::from_lookup(lookup(&[
            (ENDPOINT_CONTROL, "tcp:127.0.0.1:1234"),
            (ROOT_SECRET, &bad_secret),
        ]))
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!error.to_string().contains(&bad_secret));
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
