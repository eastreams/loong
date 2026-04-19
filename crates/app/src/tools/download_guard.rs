#[derive(Debug, Clone, Copy)]
pub(crate) struct ByteBudget {
    max_bytes: usize,
    consumed: usize,
}

impl ByteBudget {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            consumed: 0,
        }
    }

    pub(crate) fn reject_if_content_length_exceeds(
        &self,
        content_length: Option<u64>,
        surface_name: &str,
    ) -> Result<(), String> {
        let Some(content_length) = content_length else {
            return Ok(());
        };

        if content_length <= self.max_bytes as u64 {
            return Ok(());
        }

        Err(format!(
            "{surface_name} Content-Length ({content_length}) exceeds max_bytes limit ({} bytes){}",
            self.max_bytes,
            byte_budget_retry_hint(surface_name)
        ))
    }

    pub(crate) fn try_consume(&mut self, bytes: usize, surface_name: &str) -> Result<(), String> {
        let next_consumed = self.consumed.saturating_add(bytes);

        if next_consumed > self.max_bytes {
            return Err(format!(
                "{surface_name} exceeded max_bytes limit ({} bytes){}",
                self.max_bytes,
                byte_budget_retry_hint(surface_name)
            ));
        }

        self.consumed = next_consumed;
        Ok(())
    }

    pub(crate) fn consumed(&self) -> usize {
        self.consumed
    }
}

fn byte_budget_retry_hint(surface_name: &str) -> &'static str {
    if surface_name.contains("browser") {
        return "; retry with a smaller `max_bytes` or a more focused browser extract";
    }

    if surface_name.contains("web") {
        return "; retry with a smaller `max_bytes` or a narrower web request";
    }

    "; retry with a smaller `max_bytes` or a narrower read"
}

#[cfg(test)]
mod tests {
    use super::ByteBudget;

    #[test]
    fn content_length_exceed_error_includes_retry_hint() {
        let budget = ByteBudget::new(16);
        let error = budget
            .reject_if_content_length_exceeds(Some(32), "web.fetch response")
            .expect_err("content length over limit should fail");

        assert!(error.contains("max_bytes limit"));
        assert!(error.contains("narrower web request"));
    }

    #[test]
    fn streaming_overflow_error_includes_retry_hint() {
        let mut budget = ByteBudget::new(16);
        let error = budget
            .try_consume(32, "browser response")
            .expect_err("stream overrun should fail");

        assert!(error.contains("max_bytes limit"));
        assert!(error.contains("focused browser extract"));
    }
}
