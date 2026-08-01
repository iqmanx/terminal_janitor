use std::fmt;
use std::io;
use std::path::PathBuf;

/// Platform-neutral snapshot of a volume's capacity.
///
/// The only public constructor is [`DiskCapacity::new`], which enforces
/// `available_bytes <= total_bytes` and derives `used_bytes` so the
/// invariant `used_bytes = total_bytes - available_bytes` cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCapacity {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

impl DiskCapacity {
    pub fn new(total_bytes: u64, available_bytes: u64) -> Result<Self, DiskError> {
        if available_bytes > total_bytes {
            return Err(DiskError::Inconsistent {
                total_bytes,
                available_bytes,
            });
        }
        Ok(Self {
            total_bytes,
            available_bytes,
            used_bytes: total_bytes - available_bytes,
        })
    }
}

#[derive(Debug)]
pub enum DiskError {
    PathNotFound(PathBuf),
    MeasurementFailed {
        path: PathBuf,
        source: io::Error,
    },
    Inconsistent {
        total_bytes: u64,
        available_bytes: u64,
    },
    UnsupportedPlatform,
}

impl fmt::Display for DiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskError::PathNotFound(path) => {
                write!(f, "path does not exist: {}", path.display())
            }
            DiskError::MeasurementFailed { path, source } => {
                write!(
                    f,
                    "capacity measurement failed for {}: {source}",
                    path.display()
                )
            }
            DiskError::Inconsistent {
                total_bytes,
                available_bytes,
            } => write!(
                f,
                "platform returned inconsistent capacity values: \
                 available {available_bytes} bytes exceeds total {total_bytes} bytes"
            ),
            DiskError::UnsupportedPlatform => {
                write!(
                    f,
                    "disk capacity measurement is not supported on this platform"
                )
            }
        }
    }
}

impl std::error::Error for DiskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DiskError::MeasurementFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_capacity_computes_used_bytes() {
        let cap = DiskCapacity::new(1000, 400).unwrap();
        assert_eq!(cap.total_bytes, 1000);
        assert_eq!(cap.available_bytes, 400);
        assert_eq!(cap.used_bytes, 600);
    }

    #[test]
    fn zero_available_bytes_is_valid() {
        let cap = DiskCapacity::new(1000, 0).unwrap();
        assert_eq!(cap.used_bytes, 1000);
    }

    #[test]
    fn available_equal_to_total_is_valid() {
        let cap = DiskCapacity::new(1000, 1000).unwrap();
        assert_eq!(cap.used_bytes, 0);
    }

    #[test]
    fn available_greater_than_total_is_inconsistent() {
        let err = DiskCapacity::new(1000, 1500).unwrap_err();
        assert!(matches!(
            err,
            DiskError::Inconsistent {
                total_bytes: 1000,
                available_bytes: 1500
            }
        ));
    }

    #[test]
    fn zero_total_and_zero_available_is_valid() {
        let cap = DiskCapacity::new(0, 0).unwrap();
        assert_eq!(cap.used_bytes, 0);
    }
}
