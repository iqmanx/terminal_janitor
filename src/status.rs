use serde::Serialize;

use crate::config::{Config, ConfigSource};
use crate::model::DiskCapacity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageState {
    Healthy,
    Pressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageStatus {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    pub state: StorageState,
    pub config_source: ConfigSource,
}

impl StorageStatus {
    pub fn calculate(capacity: DiskCapacity, config: Config, config_source: ConfigSource) -> Self {
        let state = if capacity.available_bytes >= config.minimum_free_bytes {
            StorageState::Healthy
        } else {
            StorageState::Pressure
        };
        Self {
            total_bytes: capacity.total_bytes,
            used_bytes: capacity.used_bytes,
            available_bytes: capacity.available_bytes,
            minimum_free_bytes: config.minimum_free_bytes,
            target_free_bytes: config.target_free_bytes,
            state,
            config_source,
        }
    }

    pub fn result(self) -> &'static str {
        match self.state {
            StorageState::Healthy => "OK_NO_PRESSURE",
            StorageState::Pressure => "PRESSURE_DETECTED",
        }
    }
}

pub fn render_human(status: &StorageStatus) -> String {
    let state = match status.state {
        StorageState::Healthy => "Healthy",
        StorageState::Pressure => "Pressure",
    };
    let source = match status.config_source {
        ConfigSource::Defaults => "Defaults",
        ConfigSource::File => "File",
    };
    let mut output = format!(
        "Storage status\n──────────────\nState:         {state}\nTotal:         {}\nUsed:          {}\nAvailable:     {}\nMinimum free:  {}\nTarget free:   {}\nConfiguration: {source}\n",
        format_gib(status.total_bytes),
        format_gib(status.used_bytes),
        format_gib(status.available_bytes),
        format_gib(status.minimum_free_bytes),
        format_gib(status.target_free_bytes),
    );
    if status.state == StorageState::Pressure {
        output.push_str("\nNo cleanup was performed.\n");
    }
    output
}

pub fn render_json(status: &StorageStatus) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct JsonStatus<'a> {
        result: &'a str,
        state: &'a str,
        total_bytes: u64,
        used_bytes: u64,
        available_bytes: u64,
        minimum_free_bytes: u64,
        target_free_bytes: u64,
        config_source: &'a str,
    }

    let state = match status.state {
        StorageState::Healthy => "healthy",
        StorageState::Pressure => "pressure",
    };
    let config_source = match status.config_source {
        ConfigSource::Defaults => "defaults",
        ConfigSource::File => "file",
    };
    let json = JsonStatus {
        result: status.result(),
        state,
        total_bytes: status.total_bytes,
        used_bytes: status.used_bytes,
        available_bytes: status.available_bytes,
        minimum_free_bytes: status.minimum_free_bytes,
        target_free_bytes: status.target_free_bytes,
        config_source,
    };
    serde_json::to_string_pretty(&json).map(|mut output| {
        output.push('\n');
        output
    })
}

fn format_gib(bytes: u64) -> String {
    const GIB: f64 = (1024_u64.pow(3)) as f64;
    format!("{:.1} GiB", bytes as f64 / GIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(available: u64, minimum: u64) -> StorageStatus {
        let capacity = DiskCapacity::new(100, available).unwrap();
        let config = Config::new(minimum, minimum + 10).unwrap();
        StorageStatus::calculate(capacity, config, ConfigSource::Defaults)
    }

    #[test]
    fn healthy_at_and_above_threshold() {
        assert_eq!(status(40, 40).state, StorageState::Healthy);
        assert_eq!(status(41, 40).state, StorageState::Healthy);
    }

    #[test]
    fn pressure_below_threshold_and_at_zero() {
        assert_eq!(status(39, 40).state, StorageState::Pressure);
        assert_eq!(status(0, 40).state, StorageState::Pressure);
    }

    #[test]
    fn preserves_correct_used_space_calculation() {
        let calculated = status(25, 10);
        assert_eq!(calculated.used_bytes, 75);
    }

    #[test]
    fn json_has_stable_fields_and_values() {
        let rendered = render_json(&status(40, 40)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let object = value.as_object().unwrap();
        let fields: Vec<_> = object.keys().map(String::as_str).collect();
        assert_eq!(
            fields,
            [
                "available_bytes",
                "config_source",
                "minimum_free_bytes",
                "result",
                "state",
                "target_free_bytes",
                "total_bytes",
                "used_bytes",
            ]
        );
        assert_eq!(value["result"], "OK_NO_PRESSURE");
        assert_eq!(value["state"], "healthy");
        assert_eq!(value["config_source"], "defaults");
    }

    #[test]
    fn pressure_output_is_truthful() {
        let pressure = status(39, 40);
        assert!(render_human(&pressure).contains("No cleanup was performed."));
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&pressure).unwrap()).unwrap();
        assert_eq!(value["result"], "PRESSURE_DETECTED");
        assert_eq!(value["state"], "pressure");
    }
}
