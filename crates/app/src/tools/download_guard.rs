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
            "{surface_name} Content-Length ({content_length}) exceeds max_bytes limit ({} bytes)",
            self.max_bytes
        ))
    }

    pub(crate) fn try_consume(&mut self, bytes: usize, surface_name: &str) -> Result<(), String> {
        let next_consumed = self.consumed.saturating_add(bytes);

        if next_consumed > self.max_bytes {
            return Err(format!(
                "{surface_name} exceeded max_bytes limit ({} bytes)",
                self.max_bytes
            ));
        }

        self.consumed = next_consumed;
        Ok(())
    }

    pub(crate) fn consumed(&self) -> usize {
        self.consumed
    }
}
