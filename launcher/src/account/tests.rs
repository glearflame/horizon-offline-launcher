//! The offline names: who plays, how they are kept, and the looks they wear.
use super::*;
use std::path::PathBuf;

/// A folder of its own for the test's files.
fn nowhere() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-account-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the test folder");
    dir
}

#[test]
fn an_offline_name_is_three_to_sixteen_letters_digits_or_underscore() {
    assert!(valid_offline_name("Steve"));
    assert!(valid_offline_name("st_3"));
    assert!(valid_offline_name("a_b_c_1_2_3_4_5"));
    assert!(!valid_offline_name("ab"));
    assert!(!valid_offline_name("a".repeat(17).as_str()));
    assert!(!valid_offline_name("with space"));
    assert!(!valid_offline_name("hy-phen"));
    assert!(!valid_offline_name("accént"));
}

#[test]
fn an_offline_name_is_kept_once_and_played_as() {
    let mut accounts = Accounts::default();
    accounts.play_as(Player::Offline("Steve".into())).unwrap();
    assert_eq!(accounts.offline, vec!["Steve".to_string()]);
    assert_eq!(accounts.player(), Player::Offline("Steve".into()));
    // The same name in another case is the same name.
    accounts.play_as(Player::Offline("steve".into())).unwrap();
    assert_eq!(accounts.offline, vec!["Steve".to_string()]);
    assert_eq!(accounts.player(), Player::Offline("Steve".into()));
}

#[test]
fn the_games_own_name_plays_when_none_is_picked() {
    let mut accounts = Accounts::default();
    assert_eq!(accounts.player(), Player::GameName);
    assert_eq!(accounts.name_arg(), None);
    accounts.play_as(Player::Offline("Steve".into())).unwrap();
    assert_eq!(accounts.name_arg(), Some("Steve"));
    accounts.play_as(Player::GameName).unwrap();
    assert_eq!(accounts.player(), Player::GameName);
    assert_eq!(accounts.name_arg(), None);
}

#[test]
fn an_invalid_offline_name_is_refused_and_nothing_is_kept() {
    let mut accounts = Accounts::default();
    assert!(accounts.play_as(Player::Offline("no".into())).is_err());
    assert!(accounts.play_as(Player::Offline("way too long a name".into())).is_err());
    assert!(accounts.offline.is_empty());
}

#[test]
fn forgetting_a_name_forgets_its_look_and_stops_it_playing() {
    let mut accounts = Accounts::default();
    accounts.play_as(Player::Offline("Steve".into())).unwrap();
    accounts
        .looks
        .insert("steve".into(), crate::skins::Look::default());
    let dir = nowhere();
    accounts.remove_offline("STEVE", &dir).unwrap();
    assert!(accounts.offline.is_empty());
    assert!(accounts.looks.is_empty());
    assert_eq!(accounts.player(), Player::GameName);
    assert!(accounts.remove_offline("Steve", &dir).is_err());
}

#[test]
fn the_accounts_file_holds_only_names_and_looks() {
    let dir = nowhere();
    let path = dir.join("accounts.json");
    let mut accounts = Accounts::default();
    accounts.play_as(Player::Offline("Steve".into())).unwrap();
    accounts.save_to(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Steve"));
    // Nothing that could be a secret.
    for secret in ["token", "access_token", "refresh_token", "xuid", "client_id"] {
        assert!(!text.contains(secret), "{secret} in accounts.json");
    }
    let read = Accounts::load_from(&path);
    assert_eq!(read, accounts);
}

#[test]
fn a_missing_accounts_file_is_no_accounts() {
    let dir = nowhere();
    let accounts = Accounts::load_from(&dir.join("nothing.json"));
    assert_eq!(accounts, Accounts::default());
    assert_eq!(accounts.player(), Player::GameName);
}

#[test]
fn the_offline_uuid_is_the_one_an_offline_server_gives() {
    // `UUID.nameUUIDFromBytes("OfflinePlayer:Notch")`, as vanilla's server derives it.
    assert_eq!(
        offline_uuid("Notch"),
        "b50ad385-829d-3141-a216-7e7d7539ba7f".replace('-', "")
    );
    // The same name always is the same player.
    assert_eq!(offline_uuid("Steve"), offline_uuid("Steve"));
    assert_ne!(offline_uuid("Steve"), offline_uuid("Alex"));
}

#[test]
fn the_session_carries_the_name_and_no_file() {
    let mut accounts = Accounts::default();
    accounts.play_as(Player::Offline("Steve".into())).unwrap();
    let session = session_of(&accounts);
    assert_eq!(session.name.as_deref(), Some("Steve"));
    // Offline play needs no session file: there is no token to hand over.
    assert!(session.env.iter().all(|(k, _)| !k.contains("token")));
}
