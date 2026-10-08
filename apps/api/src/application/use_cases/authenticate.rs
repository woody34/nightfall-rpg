//! Turn a bearer token into the caller's [`AccountId`] (Stories 1.2 and 1.3). Called by the
//! gRPC auth layer for every non-public RPC; handlers never see the token.

use std::sync::Arc;

use crate::application::ports::AuthError;
use crate::application::use_cases::EnsureAccount;
use crate::application::{AppError, TokenVerifier};
use crate::domain::AccountId;

/// The authenticate use case.
pub struct Authenticate {
    verifier: Arc<dyn TokenVerifier>,
    ensure_account: EnsureAccount,
}

impl Authenticate {
    /// Builds the use case.
    #[must_use]
    pub fn new(verifier: Arc<dyn TokenVerifier>, ensure_account: EnsureAccount) -> Self {
        Self {
            verifier,
            ensure_account,
        }
    }

    /// Verifies `token`, maps its subject to an account, and records the login.
    ///
    /// # Errors
    /// `Unauthenticated` for any token problem (including a non-UUID subject);
    /// `Infrastructure` when keys or the account store are unavailable.
    pub async fn execute(&self, token: &str) -> Result<AccountId, AppError> {
        let claims = self.verifier.verify(token).await.map_err(|e| match e {
            AuthError::Unavailable(e) => AppError::Infrastructure(e),
            AuthError::Malformed
            | AuthError::Expired
            | AuthError::UnknownKey
            | AuthError::Invalid(_) => AppError::Unauthenticated(e.to_string()),
        })?;
        let account = AccountId::parse(&claims.sub)
            .map_err(|_| AppError::Unauthenticated("token subject is not an account id".into()))?;
        self.ensure_account.execute(account).await?;
        Ok(account)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use async_trait::async_trait;
    use uuid::Uuid;

    use super::*;
    use crate::application::ports::Claims;
    use crate::infrastructure::memory::{
        InMemoryAccountRepository, ManualClock, TestTokenVerifier,
    };

    fn sut(verifier: Arc<dyn TokenVerifier>) -> (Authenticate, Arc<InMemoryAccountRepository>) {
        let repo = Arc::new(InMemoryAccountRepository::default());
        let ensure = EnsureAccount::new(repo.clone(), Arc::new(ManualClock::default()));
        (Authenticate::new(verifier, ensure), repo)
    }

    #[tokio::test]
    async fn valid_token_yields_account_and_creates_it() {
        let (uc, repo) = sut(Arc::new(TestTokenVerifier));
        let id = Uuid::from_u128(42);
        let account = uc.execute(&format!("test:{id}")).await.unwrap();
        assert_eq!(account.as_uuid(), id);
        assert_eq!(repo.len(), 1);
    }

    #[tokio::test]
    async fn rejected_token_is_unauthenticated_and_writes_nothing() {
        let (uc, repo) = sut(Arc::new(TestTokenVerifier));
        let err = uc.execute("garbage").await.unwrap_err();
        assert!(matches!(err, AppError::Unauthenticated(_)), "{err:?}");
        assert_eq!(repo.len(), 0);
    }

    struct Fixed(Result<&'static str, fn() -> AuthError>);

    #[async_trait]
    impl TokenVerifier for Fixed {
        async fn verify(&self, _token: &str) -> Result<Claims, AuthError> {
            match self.0 {
                Ok(sub) => Ok(Claims {
                    sub: sub.to_owned(),
                    aud: vec![],
                    roles: vec![],
                    exp: 0,
                }),
                Err(make) => Err(make()),
            }
        }
    }

    #[tokio::test]
    async fn non_uuid_subject_is_unauthenticated() {
        let (uc, repo) = sut(Arc::new(Fixed(Ok("testplayer"))));
        let err = uc.execute("t").await.unwrap_err();
        assert!(matches!(err, AppError::Unauthenticated(_)), "{err:?}");
        assert_eq!(repo.len(), 0);
    }

    #[tokio::test]
    async fn unavailable_verifier_is_infrastructure() {
        let (uc, _) =
            sut(Arc::new(Fixed(Err(|| AuthError::Unavailable(anyhow::anyhow!("jwks down"))))));
        let err = uc.execute("t").await.unwrap_err();
        assert!(matches!(err, AppError::Infrastructure(_)), "{err:?}");
    }
}
