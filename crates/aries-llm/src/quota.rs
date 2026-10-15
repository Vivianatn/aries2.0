//! Plafond quotidien de requêtes (budget cloud) : au-delà, le fournisseur refuse, et
//! `FallbackProvider` bascule sur le modèle local. Le compteur repart à zéro à minuit
//! (heure locale) et à chaque redémarrage.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use jiff::{Zoned, civil::Date};

use crate::{ChatStream, CompletionOptions, LlmError, LlmProvider, Message, ToolSpec};

pub struct DailyQuota {
    inner: Arc<dyn LlmProvider>,
    limit: u32,
    used: Mutex<(Date, u32)>,
}

impl DailyQuota {
    pub fn new(inner: Arc<dyn LlmProvider>, limit: u32) -> Self {
        Self {
            inner,
            limit,
            used: Mutex::new((Zoned::now().date(), 0)),
        }
    }

    fn take(&self, today: Date) -> Result<(), LlmError> {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        if used.0 != today {
            *used = (today, 0);
        }
        if used.1 >= self.limit {
            return Err(LlmError::QuotaExceeded(self.limit));
        }
        used.1 += 1;
        Ok(())
    }
}

#[async_trait]
impl LlmProvider for DailyQuota {
    async fn chat_stream(
        &self,
        messages: Vec<Message>,
        tools: &[ToolSpec],
    ) -> Result<ChatStream, LlmError> {
        self.take(Zoned::now().date())?;
        self.inner.chat_stream(messages, tools).await
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        options: CompletionOptions,
    ) -> Result<String, LlmError> {
        self.take(Zoned::now().date())?;
        self.inner.complete(messages, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Never;

    #[async_trait]
    impl LlmProvider for Never {
        async fn chat_stream(
            &self,
            _: Vec<Message>,
            _: &[ToolSpec],
        ) -> Result<ChatStream, LlmError> {
            unreachable!()
        }

        async fn complete(
            &self,
            _: Vec<Message>,
            _: CompletionOptions,
        ) -> Result<String, LlmError> {
            unreachable!()
        }
    }

    #[test]
    fn refuses_past_limit_then_resets_next_day() {
        let quota = DailyQuota::new(Arc::new(Never), 2);
        let day = jiff::civil::date(2026, 10, 15);
        assert!(quota.take(day).is_ok());
        assert!(quota.take(day).is_ok());
        assert!(matches!(quota.take(day), Err(LlmError::QuotaExceeded(2))));
        assert!(quota.take(day.tomorrow().unwrap()).is_ok());
    }
}
