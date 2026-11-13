//! User settings, stored as JSON in %APPDATA%\Margin\settings.json.
//! Settings never contain typed text.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum InferenceMode {
    /// Bundled model running on this PC. Text never leaves the machine.
    Local,
    /// Any OpenAI-compatible endpoint the user configures. Text is sent there.
    Remote,
    /// Grammar rules only (Harper), no language model.
    Off,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub predictive: bool,
    pub grammar: bool,
    /// Underline mistakes in the text field (hover one to fix just that part).
    pub underline: bool,
    /// Show suggestions on the text itself (highlights, grey predictions, a small
    /// pill under a hovered word) instead of a popup list while typing.
    pub inline: bool,
    pub rewriting: bool,
    /// Mode used for automatic suggestions while typing (the shortcut shows all three).
    pub writing_mode: crate::inference::Mode,
    /// Style for Rephrase (Standard, Formal, Simple, Creative, Academic).
    pub style: crate::inference::Style,
    /// en-US, en-GB, en-CA, en-AU, en-IN (Harper dialects).
    pub language: String,
    /// Shortcut that asks for a full correction + alternative phrasings.
    pub on_demand_shortcut: String,
    /// Process names (e.g. "keepass.exe") or window-title fragments (e.g. "bank").
    pub exclusions: Vec<String>,
    pub inference: InferenceMode,
    pub local_model: String,
    pub use_gpu: bool,
    pub remote_url: String,
    pub remote_model: String,
    pub remote_key: String,
    /// Milliseconds of typing pause before suggestions are computed.
    pub debounce_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            predictive: true,
            grammar: true,
            underline: true,
            inline: true,
            rewriting: true,
            writing_mode: crate::inference::Mode::Correct,
            style: crate::inference::Style::Standard,
            language: "en-US".into(),
            on_demand_shortcut: "Ctrl+Shift+Space".into(),
            exclusions: DEFAULT_EXCLUSIONS.iter().map(|s| s.to_string()).collect(),
            inference: InferenceMode::Local,
            local_model: crate::runtime::AUTO.into(),
            use_gpu: true,
            remote_url: String::new(),
            remote_model: String::new(),
            remote_key: String::new(),
            debounce_ms: 180,
        }
    }
}

/// Apps where suggestions are unwanted or where typed text is especially sensitive.
pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "keepass.exe", "keepassxc.exe", "1password.exe", "bitwarden.exe",
    "windowsterminal.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "conhost.exe",
    "mstsc.exe", "vmconnect.exe",
];

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("Margin")
}

pub fn load() -> Settings {
    std::fs::read_to_string(dir().join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir())?;
    let tmp = dir().join("settings.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).unwrap())?;
    std::fs::rename(tmp, dir().join("settings.json"))
}

impl Settings {
    pub fn dialect(&self) -> harper_core::Dialect {
        use harper_core::Dialect::*;
        match self.language.as_str() {
            "en-GB" => British,
            "en-CA" => Canadian,
            "en-AU" => Australian,
            "en-IN" => Indian,
            _ => American,
        }
    }

    /// Is assistance disabled for this process / window title?
    pub fn is_excluded(&self, exe: &str, title: &str) -> bool {
        let exe = exe.to_lowercase();
        let title = title.to_lowercase();
        self.exclusions.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).any(|e| {
            if e.ends_with(".exe") { exe == e } else { title.contains(&e) }
        })
    }

    pub fn ai_enabled(&self) -> bool {
        self.inference != InferenceMode::Off && (self.predictive || self.rewriting || self.grammar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusions_match_exe_and_title() {
        let mut s = Settings::default();
        s.exclusions = vec!["KeePass.exe".into(), "my bank".into()];
        assert!(s.is_excluded("keepass.exe", ""));
        assert!(s.is_excluded("chrome.exe", "My Bank - Accounts - Google Chrome"));
        assert!(!s.is_excluded("chrome.exe", "Gmail"));
    }
}
