use std::path::Path;

use crate::model::{DiskCapacity, DiskError};

pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    super::measure(path)
}

pub fn file_identity(path: &Path) -> Result<(String, String), std::io::Error> {
    super::unix_file_identity(path)
}
