//! Choosing which connected Lichess account plays online.

/// One listed account: its username, if it has one, and whether it is connected.
#[derive(Clone, Copy, Debug)]
pub struct ListedAccount<'a> {
    pub username: Option<&'a str>,
    pub connected: bool,
}

/// The index of the account that plays: the connected account matching `preferred`
/// (case-insensitive), else the first connected one.
pub fn connected_account_index(
    accounts: &[ListedAccount],
    preferred: Option<&str>,
) -> Option<usize> {
    let connected: Vec<usize> = accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| account.connected)
        .map(|(i, _)| i)
        .collect();
    let wanted = preferred.map(str::to_lowercase).filter(|w| !w.is_empty());
    let matching = wanted.and_then(|wanted| {
        connected.iter().copied().find(|&i| {
            accounts[i]
                .username
                .is_some_and(|name| name.to_lowercase() == wanted)
        })
    });
    matching.or_else(|| connected.first().copied())
}
