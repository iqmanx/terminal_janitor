use std::path::Path;

use crate::model::{DiskCapacity, DiskError};

pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    super::measure(path)
}
