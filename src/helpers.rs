use std::error::Error;
use std::fs;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub(crate) current_pose_path: String,
    pub(crate) target_pos_path: String,
    pub(crate) target_vel_path: String,
    pub(crate) aim_mode_path: String,
    pub(crate) timestamp_path: String,
    pub(crate) prefix: String,
    pub(crate) team_number: u16,
    pub(crate) local_test: bool,
}

pub fn parse_json_to_config_struct(path: &str) -> Result<Settings, Box<dyn Error>>{
    let content = fs::read_to_string(path)?;
    let settings: Settings = serde_json::from_str(&content)?;
    Ok(settings)
}