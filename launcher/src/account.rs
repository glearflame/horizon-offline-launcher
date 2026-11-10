//! The player's offline names: who plays, and the look each one wears.
//!
//! Horizon plays offline. There are no accounts here, no tokens, no keychain and no network: the
//! player keeps the names they play as in `accounts.json` (names and looks, never a secret), and
//! the game starts with the name the player picked, or with the one it made up itself
//! (`horizon-player.txt`).
//!
//! The game's own session handoff (`--session-file`) is gone with the accounts that filled it:
//! there is nothing to hand over, and the client reads a name and an offline UUID from what the
//! launcher passes on the command line.
use std::path::{Path, PathBuf};

/// accounts.json: the offline names the player keeps, the one that plays, and their looks. Never
/// a token.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Accounts {
    /// Offline names the player keeps, to switch between (`--name`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offline: Vec<String>,
    /// The offline name that plays; none: the name the game made up (`horizon-player.txt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_selected: Option<String>,
    /// Each offline name's skin and cape (`skins`), by the name in lowercase.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub looks: std::collections::BTreeMap<String, crate::skins::Look>,
}

/// Who plays, as the quick switch lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Player {
    /// An offline name.
    Offline(String),
    /// Offline as the name the game made up.
    GameName,
}

/// Whether `name` can be an offline name: 3 to 16 letters, digits or `_`, as account names are.
pub fn valid_offline_name(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Accounts {
    pub(crate) fn path() -> PathBuf {
        crate::backend::data_dir().join("accounts.json")
    }
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }
    pub(crate) fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub(crate) fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|e| e.to_string())
    }

    /// Who plays now.
    pub fn player(&self) -> Player {
        match &self.offline_selected {
            Some(name) => Player::Offline(name.clone()),
            None => Player::GameName,
        }
    }

    /// Plays as `who`: an offline name (kept, to switch back to) or the game's own name.
    pub fn play_as(&mut self, who: Player) -> Result<(), String> {
        match who {
            Player::Offline(name) => {
                let name = self.add_offline(&name)?;
                self.offline_selected = Some(name);
            }
            Player::GameName => {
                self.offline_selected = None;
            }
        }
        Ok(())
    }

    /// Keeps an offline name (once, in any case); returns it as kept.
    fn add_offline(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if !valid_offline_name(name) {
            return Err(format!(
                "{name}: an offline name is 3 to 16 letters, digits or _"
            ));
        }
        if let Some(kept) = self.offline.iter().find(|n| n.eq_ignore_ascii_case(name)) {
            return Ok(kept.clone());
        }
        self.offline.push(name.to_string());
        Ok(name.to_string())
    }

    /// Forgets an offline name: its look and its skin and cape files (`skins`) go with it; if
    /// it was playing, the game's own name plays.
    pub fn remove_offline(&mut self, name: &str, skins_dir: &Path) -> Result<(), String> {
        let before = self.offline.len();
        self.offline.retain(|n| !n.eq_ignore_ascii_case(name));
        self.looks.remove(&name.to_ascii_lowercase());
        if self.offline.len() == before {
            return Err(format!("No offline name {name}."));
        }
        crate::skins::forget(skins_dir, name);
        if self
            .offline_selected
            .as_ref()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            self.offline_selected = None;
        }
        Ok(())
    }

    /// An offline name's look (the default: no skin of its own).
    pub fn look(&self, name: &str) -> crate::skins::Look {
        self.looks
            .get(&name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// The game's variables for the offline name that plays: its skin and cape.
    pub fn look_env(&self) -> Vec<(&'static str, String)> {
        match self.name_arg() {
            Some(name) => crate::skins::env(&crate::skins::dir(), name, &self.look(name)),
            None => Vec::new(),
        }
    }

    /// The game's `--name` for this run: the offline name, when one plays.
    pub fn name_arg(&self) -> Option<&str> {
        self.offline_selected.as_deref()
    }
}

/// What the game about to start gets from the accounts: the name it plays as and the look it
/// wears. There is no session file any more: offline play needs none.
#[derive(Debug, Default)]
pub struct GameSession {
    /// The name the game plays as, when the player picked one.
    pub name: Option<String>,
    /// The game's variables (the look), for the run.
    pub env: Vec<(&'static str, String)>,
}

/// The session for the game about to start: the name that plays and its look.
pub fn session_for_game() -> GameSession {
    let accounts = Accounts::load();
    session_of(&accounts)
}

/// [`session_for_game`] for one launch as `who` (`play --account`, an offline name): who plays
/// changes for this launch only, so several games can play as several names at once (#4952).
pub fn session_for_game_as(who: &Player) -> GameSession {
    let mut accounts = Accounts::load();
    let chosen = accounts.offline_selected.clone();
    let _ = accounts.play_as(who.clone());
    let session = session_of(&accounts);
    accounts.offline_selected = chosen;
    session
}

/// The name that plays and the look it wears.
fn session_of(accounts: &Accounts) -> GameSession {
    let name = accounts.name_arg().map(str::to_owned);
    let env = match &name {
        Some(name) => crate::skins::env(&crate::skins::dir(), name, &accounts.look(name)),
        None => Vec::new(),
    };
    GameSession { name, env }
}

/// The offline UUID of `name`, as an offline-mode server derives it
/// (`UUID.nameUUIDFromBytes("OfflinePlayer:<name>")`): the player the game and the servers see.
pub fn offline_uuid(name: &str) -> String {
    format!("{:032x}", horizon_platform::protocol::offline_uuid(name))
}

/// Who plays the real Minecraft: Java Edition (a Java instance): the offline name that plays,
/// or the name the game made up. The game takes the UUID an offline-mode server would give it,
/// and no token: it plays offline.
pub fn java_player() -> Result<crate::java_launch::Player, String> {
    let accounts = Accounts::load();
    let name = accounts
        .name_arg()
        .map(str::to_owned)
        .unwrap_or_else(|| "Player".to_string());
    Ok(crate::java_launch::Player {
        uuid: offline_uuid(&name),
        name,
    })
}

#[cfg(test)]
mod tests;
