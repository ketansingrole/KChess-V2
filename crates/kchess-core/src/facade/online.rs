//! Online play and challenges.

use kchess_contracts::check::{ChatText, FullFen, GameId, PresenceUsers, Uci, Username};
use kchess_contracts::core_api::OnlineApi;
use kchess_contracts::types::{
    ChallengeInfo, ChatLine, ChatRoom, DeclineReason, OngoingGame, OnlineAction, OnlineOptions,
    PresenceReport, Resumed, StartResult,
};

use crate::core::Core;
use crate::error::{CoreError, Result};

impl Core {
    /// The open challenge `id`, or the error it is no longer open.
    fn known_challenge(&self, id: &str) -> Result<ChallengeInfo> {
        let found = self.services.online().challenges().get(id);
        found.ok_or_else(|| CoreError::new("That challenge is no longer open."))
    }
}

impl OnlineApi for Core {
    async fn start_online(&self, options: OnlineOptions) -> Result<StartResult> {
        self.services.online().start(&options).await
    }

    async fn resume_online(&self) -> Result<Option<Resumed>> {
        self.services.online().resume().await
    }

    async fn cancel_online(&self) -> Result<()> {
        self.services.online().cancel();
        Ok(())
    }

    async fn play_online(&self, id: GameId, r#move: Uci) -> Result<()> {
        self.services.online().move_uci(&id.0, &r#move.0).await
    }

    async fn play_correspondence(
        &self,
        account: Username,
        id: GameId,
        fen: FullFen,
        r#move: Uci,
    ) -> Result<()> {
        let online = self.services.online();
        online
            .move_correspondence(&account.0, &id.0, &fen.0, &r#move.0)
            .await
    }

    async fn online_action(&self, id: GameId, action: OnlineAction) -> Result<()> {
        self.services.online().action(&id.0, action).await
    }

    async fn online_chat(&self, id: GameId) -> Result<Vec<ChatLine>> {
        self.services.online().chat(&id.0).await
    }

    async fn send_chat(&self, id: GameId, room: ChatRoom, text: ChatText) -> Result<()> {
        let online = self.services.online();
        online.send_chat(&id.0, room.as_str(), &text.0).await
    }

    async fn presence(&self, usernames: PresenceUsers) -> Result<PresenceReport> {
        self.services.online().presence(&usernames.0).await
    }

    async fn challenges(&self) -> Result<Vec<ChallengeInfo>> {
        Ok(self.services.online().challenges().list())
    }

    async fn accept_challenge(&self, id: GameId) -> Result<()> {
        let challenge = self.known_challenge(&id.0)?;
        if challenge.direction != "in" || !challenge.playable {
            let problem = challenge
                .problem
                .as_deref()
                .unwrap_or("Only challenges sent to you can be accepted.");
            return Err(CoreError::new(problem));
        }
        self.services.online().accept_challenge(&challenge).await?;
        self.services.online().challenges().remove(&id.0);
        Ok(())
    }

    async fn decline_challenge(&self, id: GameId, reason: DeclineReason) -> Result<()> {
        let challenge = self.known_challenge(&id.0)?;
        if challenge.direction != "in" {
            return Err(CoreError::new("Withdraw your own challenge instead."));
        }
        let online = self.services.online();
        online
            .decline_challenge(&challenge, reason.as_str())
            .await?;
        online.challenges().remove(&id.0);
        Ok(())
    }

    async fn cancel_challenge(&self, id: GameId) -> Result<()> {
        let challenge = self.known_challenge(&id.0)?;
        if challenge.direction != "out" {
            return Err(CoreError::new("Decline a challenge sent to you instead."));
        }
        self.services
            .online()
            .withdraw_challenge(&challenge)
            .await?;
        self.services.online().challenges().remove(&id.0);
        Ok(())
    }

    async fn ongoing_games(&self) -> Result<Vec<OngoingGame>> {
        self.services.online().ongoing().await
    }

    async fn open_game(&self, account: Username, id: GameId) -> Result<()> {
        self.services.online().open(&account.0, &id.0).await
    }
}
