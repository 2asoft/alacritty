use super::{Action, Command, GraphicsError, ProcessedCommand, process_command};

#[derive(Debug)]
pub enum GraphicsRequest {
    Command(Result<Command, GraphicsError>),
    Chunked { command: Command, chunks: Vec<Vec<u8>> },
}

impl GraphicsRequest {
    pub fn command(&self) -> Option<&Command> {
        match self {
            Self::Command(Ok(command)) | Self::Chunked { command, .. } => Some(command),
            Self::Command(Err(_)) => None,
        }
    }

    pub fn anchor(&self) -> Option<crate::index::Point> {
        match self {
            Self::Command(Ok(command)) | Self::Chunked { command, .. } => command.anchor,
            Self::Command(Err(_)) => None,
        }
    }
}

#[derive(Debug)]
pub struct PendingTransmission {
    command: Command,
    chunks: Vec<Vec<u8>>,
    encoded_bytes: usize,
}

impl PendingTransmission {
    pub fn start(mut command: Command, encoded_limit: usize) -> Result<Self, GraphicsError> {
        let encoded_bytes = command.payload.len();
        if encoded_bytes % 4 != 0 {
            return Err(GraphicsError::Invalid);
        }
        if encoded_bytes > encoded_limit {
            return Err(GraphicsError::PayloadTooLarge);
        }
        let first = std::mem::take(&mut command.payload);
        Ok(Self { command, chunks: vec![first], encoded_bytes })
    }

    pub fn push(
        mut self,
        continuation: Command,
        encoded_limit: usize,
    ) -> Result<PendingResult, GraphicsError> {
        if !continuation.is_valid_continuation(self.command.action) {
            return Err(GraphicsError::Invalid);
        }
        if continuation.more == Some(true) && continuation.payload.len() % 4 != 0 {
            return Err(GraphicsError::Invalid);
        }
        self.encoded_bytes = self
            .encoded_bytes
            .checked_add(continuation.payload.len())
            .ok_or(GraphicsError::TooLarge)?;
        if self.encoded_bytes > encoded_limit {
            return Err(GraphicsError::PayloadTooLarge);
        }
        self.chunks.push(continuation.payload);
        if continuation.more == Some(true) {
            Ok(PendingResult::Pending(self))
        } else {
            self.command.more = Some(false);
            self.command.quiet = continuation.quiet.or(self.command.quiet);
            self.command.anchor = continuation.anchor;
            Ok(PendingResult::Complete(GraphicsRequest::Chunked {
                command: self.command,
                chunks: self.chunks,
            }))
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
        let valid_action = matches!(
            (initial_action, self.action),
            (Some(Action::TransmitFrame), Some(Action::TransmitFrame)) | (_, None)
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
    match request {
        GraphicsRequest::Command(command) => {
            process_command(command, storage_limit, local_transmission)
        },
        GraphicsRequest::Chunked { mut command, chunks } => {
            let Some(encoded_bytes) =
                chunks.iter().try_fold(0usize, |total, chunk| total.checked_add(chunk.len()))
            else {
                return ProcessedCommand::Error {
                    command: Some(command),
                    error: GraphicsError::TooLarge,
                };
            };
            command.payload = Vec::with_capacity(encoded_bytes);
            for chunk in chunks {
                command.payload.extend_from_slice(&chunk);
            }
            process_command(Ok(command), storage_limit, local_transmission)
        },
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as Base64;

    use super::*;
    use crate::graphics::Format;

    #[test]
    fn assembles_chunks_before_decoding() {
        let bytes = Base64.encode([1, 2, 3, 4]);
        let split = 4;
        let first = Command {
            format: Some(Format::Rgba),
            width: Some(1),
            height: Some(1),
            more: Some(true),
            payload: bytes.as_bytes()[..split].to_vec(),
            ..Default::default()
        };
        let continuation = Command {
            more: Some(false),
            payload: bytes.as_bytes()[split..].to_vec(),
            ..Default::default()
        };
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
    fn animation_chunks_accept_kitty_and_explicit_frame_continuations() {
        for action in [None, Some(Action::TransmitFrame)] {
            let first = Command {
                action: Some(Action::TransmitFrame),
                image_id: Some(1),
                more: Some(true),
                quiet: Some(0),
                payload: b"AAAA".to_vec(),
                ..Default::default()
            };
            let continuation = Command {
                action,
                more: Some(false),
                quiet: Some(1),
                payload: b"AAAA".to_vec(),
                ..Default::default()
            };
            let pending = PendingTransmission::start(first, 100).unwrap();
            let PendingResult::Complete(GraphicsRequest::Chunked { command, .. }) =
                pending.push(continuation, 100).unwrap()
            else {
                panic!("expected complete frame transmission");
            };
            assert_eq!(command.action, Some(Action::TransmitFrame));
            assert_eq!(command.image_id, Some(1));
            assert_eq!(command.quiet, Some(1));
        }
    }

    #[test]
    fn rejects_invalid_nonfinal_base64_boundary() {
        let first = Command { more: Some(true), payload: b"AAA".to_vec(), ..Default::default() };
        assert_eq!(PendingTransmission::start(first, 100).unwrap_err(), GraphicsError::Invalid);
    }

    #[test]
    fn rejects_metadata_in_continuation() {
        let first = Command { more: Some(true), payload: b"AAAA".to_vec(), ..Default::default() };
        let continuation = Command {
            image_id: Some(1),
            more: Some(false),
            payload: b"AAAA".to_vec(),
            ..Default::default()
        };
        let pending = PendingTransmission::start(first, 100).unwrap();
        assert_eq!(pending.push(continuation, 100).unwrap_err(), GraphicsError::Invalid);
    }
}
