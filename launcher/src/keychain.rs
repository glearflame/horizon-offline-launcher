//! The system keychain: where the launcher keeps the secrets that are not the player's game
//! files — the Tailscale access token, for now. Nothing here is about a game account: Horizon
//! plays offline and has no account to keep.
//!
//! The OS keychain if it answers (GNOME Keyring or KWallet through the Secret Service on Linux
//! and FreeBSD, the Keychain on macOS, Credential Manager on Windows), else memory for this run
//! only, and the player is told so.

// A unit test never reaches the player's keychain (`keychain_or_why`), so the store and the
// bounded calls around it are dead code there.
#![cfg_attr(test, allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The keychain service name every secret is stored under.
pub const SERVICE: &str = "dev.playhorizon.launcher";

/// Where secrets are kept: the OS keychain, or memory when there is none.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    /// False for memory: secrets are gone when the launcher closes.
    fn persistent(&self) -> bool;
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn persistent(&self) -> bool {
        false
    }
}

/// How long a call to the system keyring may take before the launcher gives up on it (#12179). An
/// unlock prompt the player is typing a password into counts as an answer still coming, so this is
/// longer than a working keyring ever takes, short enough that the launcher says what is wrong.
pub const KEYCHAIN_WAIT: Duration = Duration::from_secs(20);

/// What the launcher says when the system keyring didn't answer within `wait`.
pub fn keyring_no_answer(wait: Duration) -> String {
    crate::i18n::trf(
        "Your system keyring didn't answer within {seconds} seconds. Make sure it is running and unlocked, then restart the launcher.",
        &[("seconds", &wait.as_secs().max(1).to_string())],
    )
}

/// One call to a [`Bounded`] keyring: its answer, or that the caller gave up on it.
struct Call<T> {
    answer: Option<T>,
    gave_up: bool,
}

/// A keyring whose calls give up after `wait` (#12179): a Secret Service that never answers (a
/// stuck KWallet or GNOME Keyring daemon) otherwise holds the call forever. Each call runs on a
/// thread of its own; one given up on keeps running there, and while it does, later calls fail at
/// once instead of piling up behind it.
pub struct Bounded {
    keyring: std::sync::Arc<dyn SecretStore>,
    wait: Duration,
    /// Calls given up on that haven't come back yet.
    stuck: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Bounded {
    pub fn new(keyring: impl SecretStore + 'static, wait: Duration) -> Self {
        Self {
            keyring: std::sync::Arc::new(keyring),
            wait,
            stuck: Default::default(),
        }
    }

    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn SecretStore) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        use std::sync::atomic::Ordering::SeqCst;
        let no_answer = || {
            let said = keyring_no_answer(self.wait);
            eprintln!("Horizon: {said}");
            Err(said)
        };
        if self.stuck.load(SeqCst) > 0 {
            return no_answer();
        }
        let call = std::sync::Arc::new((
            Mutex::new(Call::<Result<T, String>> {
                answer: None,
                gave_up: false,
            }),
            std::sync::Condvar::new(),
        ));
        let (keyring, stuck, theirs) = (self.keyring.clone(), self.stuck.clone(), call.clone());
        let spawned = std::thread::Builder::new()
            .name("keyring".into())
            .spawn(move || {
                let answer = f(keyring.as_ref());
                let (slot, done) = &*theirs;
                let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
                if slot.gave_up {
                    stuck.fetch_sub(1, SeqCst);
                } else {
                    slot.answer = Some(answer);
                    done.notify_one();
                }
            });
        if let Err(e) = spawned {
            return Err(e.to_string());
        }
        let (slot, done) = &*call;
        let slot = slot.lock().unwrap_or_else(|e| e.into_inner());
        let (mut slot, _) = done
            .wait_timeout_while(slot, self.wait, |call| call.answer.is_none())
            .unwrap_or_else(|e| e.into_inner());
        if let Some(answer) = slot.answer.take() {
            return answer;
        }
        // Under the call's lock: the thread, when it ends, sees this and counts itself back.
        slot.gave_up = true;
        self.stuck.fetch_add(1, SeqCst);
        drop(slot);
        no_answer()
    }
}

impl SecretStore for Bounded {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let key = key.to_string();
        self.call(move |keyring| keyring.get(&key))
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let (key, secret) = (key.to_string(), secret.to_string());
        self.call(move |keyring| keyring.set(&key, &secret))
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let key = key.to_string();
        self.call(move |keyring| keyring.delete(&key))
    }
    fn persistent(&self) -> bool {
        self.keyring.persistent()
    }
}

/// The Secret Service (GNOME Keyring, KWallet) through oo7, as gpui itself uses it.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub struct SystemStore;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl SystemStore {
    fn keyring() -> Result<oo7::Keyring, String> {
        futures_lite::future::block_on(async {
            // `oo7::Keyring::new` takes `GTK_USE_PORTAL=1` (set by many KDE desktops) for a
            // sandbox, and then keeps the secrets in a file under a key the Secret portal gives
            // per app id. Outside a sandbox that id comes from how the launcher was started (the
            // menu or a terminal), so the file opened in one start and not the next (#3249).
            // Flatpak (`/.flatpak-info`) and Snap (`$SNAP`) keep the portal's file.
            let sandboxed =
                Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some();
            let keyring = if sandboxed {
                oo7::Keyring::new().await.map_err(|e| e.to_string())?
            } else {
                let service = oo7::dbus::Service::new().await.map_err(|e| e.to_string())?;
                oo7::Keyring::DBus(
                    service
                        .default_collection()
                        .await
                        .map_err(|e| e.to_string())?,
                )
            };
            keyring.unlock().await.map_err(|e| e.to_string())?;
            Ok(keyring)
        })
    }
}
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl SecretStore for SystemStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            let items = keyring
                .search_items(&attributes)
                .await
                .map_err(|e| e.to_string())?;
            let Some(item) = items.into_iter().next() else {
                return Ok(None);
            };
            item.unlock().await.map_err(|e| e.to_string())?;
            let secret = item.secret().await.map_err(|e| e.to_string())?;
            Ok(String::from_utf8(secret.to_vec()).ok())
        })
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            keyring
                .create_item(
                    "Horizon launcher",
                    &attributes,
                    secret.as_bytes(),
                    true,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            keyring.delete(&attributes).await.map_err(|e| e.to_string())
        })
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// Keychain on macOS, Credential Manager on Windows (the keyring crate).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct SystemStore;

/// One keychain item per key, the whole secret in it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
trait Items {
    fn read(&self, key: &str) -> Result<Option<String>, String>;
    fn write(&self, key: &str, piece: &str) -> Result<(), String>;
    /// Removes one item; false when there was none.
    fn remove(&self, key: &str) -> Result<bool, String>;
}

/// The OS keychain's items, under [`SERVICE`].
#[cfg(any(target_os = "macos", target_os = "windows"))]
struct KeyringItems;

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Items for KeyringItems {
    fn read(&self, key: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn write(&self, key: &str, piece: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(piece))
            .map_err(|e| e.to_string())
    }
    fn remove(&self, key: &str) -> Result<bool, String> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl SecretStore for SystemStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(get_item(&KeyringItems, key))
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        set_item(&KeyringItems, key, secret)
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        delete_item(&KeyringItems, key)
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// One item per key, the whole secret in it. A secret an older launcher split over `key.1`,
/// `key.2`, ... reads whole and folds into the one item, so a read touches a single item — and asks
/// once — from then on.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn get_item(items: &dyn Items, key: &str) -> Option<String> {
    let mut secret = items.read(key).ok().flatten()?;
    let mut split = false;
    for n in 1.. {
        match items.read(&piece_key(key, n)).ok().flatten() {
            Some(piece) => {
                secret.push_str(&piece);
                split = true;
            }
            None => break,
        }
    }
    if split {
        let _ = set_item(items, key, &secret);
    }
    Some(secret)
}

/// Writes `secret` to `key` and removes `key.1`, `key.2`, ... pieces an older, split secret left:
/// they would read back as part of this one.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn set_item(items: &dyn Items, key: &str, secret: &str) -> Result<(), String> {
    items.write(key, secret)?;
    for n in 1.. {
        if !items.remove(&piece_key(key, n))? {
            break;
        }
    }
    Ok(())
}

/// Removes `key` and the pieces an older, split secret left beside it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn delete_item(items: &dyn Items, key: &str) -> Result<(), String> {
    for n in 0.. {
        if !items.remove(&piece_key(key, n))? {
            break;
        }
    }
    Ok(())
}

/// `key` for the piece `n`: the first is the key itself, the rest `key.1`, `key.2`, ...
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn piece_key(key: &str, n: usize) -> String {
    if n == 0 {
        key.to_string()
    } else {
        format!("{key}.{n}")
    }
}

/// What the launcher says when the system keychain didn't answer: the secret lasts only this run.
pub const NO_KEYCHAIN: &str = "No keychain available: the token is kept until the launcher closes.";

/// What the keychain answered when [`keychain`] fell back to memory.
static NO_KEYCHAIN_WHY: OnceLock<String> = OnceLock::new();

/// What the window and the CLI say when a secret lasts only this run: [`NO_KEYCHAIN`] and why.
/// None while the keychain keeps it.
pub fn no_keychain() -> Option<String> {
    NO_KEYCHAIN_WHY.get().map(|why| format!("{NO_KEYCHAIN} ({why})"))
}

/// The OS keychain if it answers, else why not.
fn keychain_or_why() -> Result<Box<dyn SecretStore>, String> {
    // Unit tests never reach the player's keychain.
    #[cfg(all(
        not(test),
        any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "macos",
            target_os = "windows"
        )
    ))]
    {
        if std::env::var_os("HORIZON_NO_KEYCHAIN").is_some() {
            return Err("HORIZON_NO_KEYCHAIN is set.".into());
        }
        let keychain = Bounded::new(SystemStore, KEYCHAIN_WAIT);
        keychain.get("probe").map(|_| Box::new(keychain) as _)
    }
    #[cfg(not(all(
        not(test),
        any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "macos",
            target_os = "windows"
        )
    )))]
    Err("This system has no keychain.".into())
}

/// Secrets never written to a file of the launcher's own, the Tailscale access token (it
/// administers the player's whole tailnet): the keychain if it answers, else memory for this run.
pub fn keychain() -> &'static dyn SecretStore {
    static KEYCHAIN: OnceLock<Box<dyn SecretStore>> = OnceLock::new();
    KEYCHAIN
        .get_or_init(|| {
            keychain_or_why().unwrap_or_else(|why| {
                let _ = NO_KEYCHAIN_WHY.set(why);
                Box::<MemoryStore>::default()
            })
        })
        .as_ref()
}
