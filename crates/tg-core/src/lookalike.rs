use std::collections::HashSet;
use std::sync::Mutex;

/// Most look-alike usernames looked up for one username.
const MAX_LOOKUPS: usize = 3;

/// What a username can swap in for a letter, and the letter. `1` stands for `l` and for `i`.
const POSING: [(&str, &str); 5] = [("0", "o"), ("1", "l"), ("1", "i"), ("rn", "m"), ("vv", "w")];

/// Folds case, punctuation and look-alike characters, so "a1ice_smith_" and "Alice Smith" compare equal.
pub fn skeleton(text: &str) -> String {
    let text = text.to_lowercase().replace("rn", "m").replace("vv", "w");
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| match c {
            '0' => 'o',
            '1' | 'i' => 'l',
            c => c,
        })
        .collect()
}

/// Usernames `username` could pose as by its [`POSING`] swaps: each swapped everywhere, then one at a time, at most
/// [`MAX_LOOKUPS`]. Most usernames have no such swap, so none.
fn posed_usernames(username: &str) -> Vec<String> {
    let lowercase = username.to_ascii_lowercase();
    let username = lowercase.as_str();
    let everywhere =
        ["l", "i"].map(|one| username.replace('0', "o").replace('1', one).replace("rn", "m").replace("vv", "w"));
    let one_at_a_time = POSING.iter().flat_map(|(posing, letter)| {
        username
            .match_indices(posing)
            .map(move |(at, _)| format!("{}{letter}{}", &username[..at], &username[at + posing.len()..]))
    });
    let mut posed = Vec::new();
    for candidate in everywhere.into_iter().chain(one_at_a_time) {
        if candidate != username && !posed.contains(&candidate) {
            posed.push(candidate);
        }
    }
    posed.truncate(MAX_LOOKUPS);
    posed
}

/// Usernames already checked for look-alikes this session, so Telegram is asked about each one once.
#[derive(Default)]
pub(crate) struct Checked(Mutex<HashSet<String>>);

impl Checked {
    /// The usernames to look up for `username`, the first time only.
    pub(crate) fn lookups(&self, username: &str) -> Vec<String> {
        match self.0.lock().expect("checked usernames lock poisoned").insert(username.to_ascii_lowercase()) {
            true => posed_usernames(username),
            false => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeletons_fold_look_alike_names_and_handles() {
        assert_eq!(skeleton("Alice Smith"), skeleton("alice_smith_"));
        assert_eq!(skeleton("a1ice.smith"), skeleton("AIice Smith"));
        assert_eq!(skeleton("B0b Jones"), skeleton("bobjones"));
        assert_eq!(skeleton("Mike"), skeleton("rnike"));
        assert_eq!(skeleton("walter_white"), skeleton("vvalter_vvhite"));
        assert_ne!(skeleton("Alice Smith"), skeleton("Alice Smyth"));
        assert_eq!(skeleton("\u{1F60A} ???"), "");
    }

    #[test]
    fn usernames_with_swapped_in_digits_or_letters_are_looked_up_as_the_original() {
        assert_eq!(posed_usernames("A1chemist"), ["alchemist", "aichemist"]);
        assert_eq!(posed_usernames("dur0v"), ["durov"]);
        assert_eq!(posed_usernames("rnike_vvhite"), ["mike_white", "mike_vvhite", "rnike_white"]);
        assert_eq!(posed_usernames("b1nance_0fficial"), ["blnance_official", "binance_official", "b1nance_official"]);
        assert_eq!(posed_usernames("alice_smith"), [] as [&str; 0], "most usernames need no lookup");
        for username in ["A1chemist", "dur0v", "rnike_vvhite", "b1nance_0fficial"] {
            for posed in posed_usernames(username) {
                assert_eq!(skeleton(&posed), skeleton(username), "{posed} is flagged as {username}'s look-alike");
            }
        }
    }

    #[test]
    fn each_username_is_looked_up_once_per_session() {
        let checked = Checked::default();
        assert_eq!(checked.lookups("dur0v"), ["durov"]);
        assert_eq!(checked.lookups("DUR0V"), [] as [&str; 0]);
        assert_eq!(checked.lookups("alice_smith"), [] as [&str; 0]);
    }
}
