use super::{Action, Command, GraphicsError, ParsedCommand, ProcessedCommand, process_command};

// Coalesce wire chunks so metadata depends on encoded bytes, never on APC count.
const ENCODED_BLOCK_BYTES: usize = 128 * 1024;

#[derive(Debug)]
pub enum EncodedPayload {
    Single(Vec<u8>),
    Chunks(Vec<Vec<u8>>),
}

#[derive(Debug)]
pub enum GraphicsRequest {
    Invalid { command: Option<Command>, error: GraphicsError },
    Command { command: Command, payload: EncodedPayload },
}

impl GraphicsRequest {
    pub fn command(&self) -> Option<&Command> {
        match self {
            Self::Command { command, .. } => Some(command),
            Self::Invalid { command, .. } => command.as_ref(),
        }
    }
}

#[derive(Debug)]
pub struct PendingTransmission {
    command: Command,
    chunks: Vec<Vec<u8>>,
    encoded_bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RequestError {
    pub command: Option<Box<Command>>,
    pub error: GraphicsError,
}

impl RequestError {
    pub(crate) fn into_request(self) -> GraphicsRequest {
        GraphicsRequest::Invalid {
            command: self.command.map(|command| *command),
            error: self.error,
        }
    }
}

impl PendingTransmission {
    pub(crate) fn start(parsed: ParsedCommand, encoded_limit: usize) -> Result<Self, RequestError> {
        let ParsedCommand { command, payload } = parsed;
        let encoded_bytes = payload.len();
        if encoded_bytes % 4 != 0 {
            return Err(RequestError {
                command: Some(Box::new(command)),
                error: GraphicsError::Invalid,
            });
        }
        if encoded_bytes > encoded_limit {
            return Err(RequestError {
                command: Some(Box::new(command)),
                error: GraphicsError::PayloadTooLarge,
            });
        }
        let mut pending = Self { command, chunks: Vec::new(), encoded_bytes };
        pending.append_payload(&payload);
        Ok(pending)
    }

    pub(crate) fn push(
        mut self,
        parsed: ParsedCommand,
        encoded_limit: usize,
    ) -> Result<PendingResult, RequestError> {
        let ParsedCommand { command: continuation, payload } = parsed;
        if !continuation.is_valid_continuation(self.command.action) {
            return Err(RequestError {
                command: Some(Box::new(self.command)),
                error: GraphicsError::Invalid,
            });
        }
        if continuation.more == Some(true) && payload.len() % 4 != 0 {
            return Err(RequestError {
                command: Some(Box::new(self.command)),
                error: GraphicsError::Invalid,
            });
        }
        let Some(encoded_bytes) = self.encoded_bytes.checked_add(payload.len()) else {
            return Err(RequestError {
                command: Some(Box::new(self.command)),
                error: GraphicsError::TooLarge,
            });
        };
        self.encoded_bytes = encoded_bytes;
        if self.encoded_bytes > encoded_limit {
            return Err(RequestError {
                command: Some(Box::new(self.command)),
                error: GraphicsError::PayloadTooLarge,
            });
        }
        self.append_payload(&payload);
        if continuation.more == Some(true) {
            Ok(PendingResult::Pending(self))
        } else {
            self.command.more = Some(false);
            self.command.quiet = continuation.quiet.or(self.command.quiet);
            Ok(PendingResult::Complete(GraphicsRequest::Command {
                command: self.command,
                payload: EncodedPayload::Chunks(self.chunks),
            }))
        }
    }

    fn append_payload(&mut self, mut payload: &[u8]) {
        while !payload.is_empty() {
            if self.chunks.last().is_none_or(|block| block.len() == ENCODED_BLOCK_BYTES) {
                self.chunks.push(Vec::with_capacity(ENCODED_BLOCK_BYTES));
            }
            let Some(block) = self.chunks.last_mut() else {
                return;
            };
            let count = payload.len().min(ENCODED_BLOCK_BYTES - block.len());
            block.extend_from_slice(&payload[..count]);
            payload = &payload[count..];
        }
    }
}

#[derive(Debug)]
pub enum PendingResult {
    Pending(PendingTransmission),
    Complete(GraphicsRequest),
}

impl Command {
    fn is_valid_continuation(&self, initial_action: Option<Action>) -> bool {
        let valid_action = self.action.is_none()
            || self.action == initial_action
                && matches!(
                    initial_action,
                    Some(Action::Transmit | Action::TransmitAndPlace | Action::TransmitFrame)
                );
        valid_action
            && self.format.is_none()
            && self.transmission.is_none()
            && self.compression.is_none()
            && self.delete.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.data_size.is_none()
            && self.data_offset.is_none()
            && self.image_id.is_none()
            && self.image_number.is_none()
            && self.placement_id.is_none()
            && self.usage.is_none()
            && self.x.is_none()
            && self.y.is_none()
            && self.crop_width.is_none()
            && self.crop_height.is_none()
            && self.x_offset.is_none()
            && self.y_offset.is_none()
            && self.columns.is_none()
            && self.rows.is_none()
            && self.cursor_policy.is_none()
            && self.unicode_placeholder.is_none()
            && self.z_index.is_none()
            && self.parent_image_id.is_none()
            && self.parent_placement_id.is_none()
            && self.horizontal_offset.is_none()
            && self.vertical_offset.is_none()
    }
}

pub fn process_request(
    request: GraphicsRequest,
    storage_limit: usize,
    local_transmission: bool,
) -> ProcessedCommand {
    let (command, payload) = match request {
        GraphicsRequest::Invalid { command, error } => {
            return ProcessedCommand::Error { command, error };
        },
        GraphicsRequest::Command { command, payload: EncodedPayload::Single(payload) } => {
            (command, payload)
        },
        GraphicsRequest::Command { command, payload: EncodedPayload::Chunks(chunks) } => {
            let Some(encoded_bytes) =
                chunks.iter().try_fold(0usize, |total, chunk| total.checked_add(chunk.len()))
            else {
                return ProcessedCommand::Error {
                    command: Some(command),
                    error: GraphicsError::TooLarge,
                };
            };
            let mut payload = Vec::with_capacity(encoded_bytes);
            for chunk in chunks {
                payload.extend_from_slice(&chunk);
            }
            (command, payload)
        },
    };
    process_command(command, payload, storage_limit, local_transmission)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as Base64;

    use super::*;
    use crate::graphics::Format;

    fn parsed(command: Command, payload: impl Into<Vec<u8>>) -> ParsedCommand {
        ParsedCommand { command, payload: payload.into() }
    }

    #[test]
    fn empty_and_small_continuations_bound_metadata_by_encoded_bytes() {
        let first = parsed(
            Command { image_id: Some(1), more: Some(true), ..Default::default() },
            Vec::new(),
        );
        let mut pending = PendingTransmission::start(first, ENCODED_BLOCK_BYTES + 4).unwrap();
        for _ in 0..100_000 {
            let continuation =
                parsed(Command { more: Some(true), ..Default::default() }, Vec::new());
            let PendingResult::Pending(next) =
                pending.push(continuation, ENCODED_BLOCK_BYTES + 4).unwrap()
            else {
                panic!("unexpected completion");
            };
            pending = next;
        }
        assert!(pending.chunks.is_empty());
        for _ in 0..ENCODED_BLOCK_BYTES / 4 + 1 {
            let continuation =
                parsed(Command { more: Some(true), ..Default::default() }, b"AAAA".to_vec());
            let PendingResult::Pending(next) =
                pending.push(continuation, ENCODED_BLOCK_BYTES + 4).unwrap()
            else {
                panic!("unexpected completion");
            };
            pending = next;
        }
        assert_eq!(pending.chunks.len(), 2);
        assert_eq!(pending.chunks[0].len(), ENCODED_BLOCK_BYTES);
        assert_eq!(pending.chunks[1].len(), 4);
        assert!(pending.chunks.iter().all(|block| block.capacity() <= ENCODED_BLOCK_BYTES));
        assert_eq!(pending.encoded_bytes, ENCODED_BLOCK_BYTES + 4);
    }

    #[test]
    fn assembles_chunks_before_decoding() {
        let bytes = Base64.encode([1, 2, 3, 4]);
        let split = 4;
        let first = parsed(
            Command {
                format: Some(Format::Rgba),
                width: Some(1),
                height: Some(1),
                more: Some(true),
                ..Default::default()
            },
            bytes.as_bytes()[..split].to_vec(),
        );
        let continuation = parsed(
            Command { more: Some(false), ..Default::default() },
            bytes.as_bytes()[split..].to_vec(),
        );
        let pending = PendingTransmission::start(first, 100).unwrap();
        let PendingResult::Complete(request) = pending.push(continuation, 100).unwrap() else {
            panic!("expected complete transmission");
        };

        match process_request(request, 4, true) {
            ProcessedCommand::Decoded { image, .. } => assert_eq!(image.bytes(), &[1, 2, 3, 4]),
            result => panic!("unexpected result: {result:?}"),
        }
    }

    #[test]
    fn chunks_accept_kitty_action_repetition_and_final_quiet_override() {
        for (initial_action, continuation_actions) in [
            (Action::TransmitFrame, [None, Some(Action::TransmitFrame)]),
            (Action::TransmitAndPlace, [None, Some(Action::TransmitAndPlace)]),
        ] {
            for action in continuation_actions {
                let first = parsed(
                    Command {
                        action: Some(initial_action),
                        image_id: Some(1),
                        more: Some(true),
                        quiet: Some(0),
                        ..Default::default()
                    },
                    b"AAAA".to_vec(),
                );
                let continuation = parsed(
                    Command { action, more: Some(false), quiet: Some(1), ..Default::default() },
                    b"AAAA".to_vec(),
                );
                let pending = PendingTransmission::start(first, 100).unwrap();
                let PendingResult::Complete(GraphicsRequest::Command { command, .. }) =
                    pending.push(continuation, 100).unwrap()
                else {
                    panic!("expected complete transmission");
                };
                assert_eq!(command.action, Some(initial_action));
                assert_eq!(command.image_id, Some(1));
                assert_eq!(command.quiet, Some(1));
            }
        }
    }

    #[test]
    fn rejects_invalid_nonfinal_base64_boundary_with_command_identity() {
        let first = parsed(
            Command { image_id: Some(7), more: Some(true), ..Default::default() },
            b"AAA".to_vec(),
        );
        assert_eq!(PendingTransmission::start(first, 100).unwrap_err(), RequestError {
            command: Some(Box::new(Command {
                image_id: Some(7),
                more: Some(true),
                ..Default::default()
            })),
            error: GraphicsError::Invalid,
        });
    }

    #[test]
    fn rejects_metadata_in_continuation_with_initial_identity() {
        let first = parsed(
            Command { image_id: Some(9), more: Some(true), ..Default::default() },
            b"AAAA".to_vec(),
        );
        let continuation = parsed(
            Command { image_id: Some(1), more: Some(false), ..Default::default() },
            b"AAAA".to_vec(),
        );
        let pending = PendingTransmission::start(first, 100).unwrap();
        assert_eq!(pending.push(continuation, 100).unwrap_err(), RequestError {
            command: Some(Box::new(Command {
                image_id: Some(9),
                more: Some(true),
                ..Default::default()
            })),
            error: GraphicsError::Invalid,
        });
    }
}
