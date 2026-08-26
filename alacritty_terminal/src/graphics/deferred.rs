use super::{
    Command, FrameWork, GraphicsError, GraphicsRequest, PixelBuffer, PreparedFrameMutation,
    process_request,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProcessingOptions {
    pub decode_limit: usize,
    pub local_transmission: bool,
}

pub(crate) enum DeferredGraphics {
    Decode(DecodeWork),
    Compose(FrameWork),
}

pub(crate) enum PreparedGraphics {
    Command(ProcessedCommand),
    Frame(PreparedFrameMutation),
}

pub(crate) struct DecodeWork {
    request: GraphicsRequest,
    options: ProcessingOptions,
}

#[derive(Debug)]
pub(crate) enum ProcessedCommand {
    Decoded { command: Command, image: PixelBuffer },
    Metadata(Command),
    Error { command: Option<Command>, error: GraphicsError },
}

impl DeferredGraphics {
    pub(crate) fn decode(request: GraphicsRequest, options: ProcessingOptions) -> Self {
        Self::Decode(DecodeWork { request, options })
    }

    pub(crate) fn process(self) -> PreparedGraphics {
        match self {
            Self::Decode(work) => PreparedGraphics::Command(process_request(
                work.request,
                work.options.decode_limit,
                work.options.local_transmission,
            )),
            Self::Compose(work) => match work.process() {
                Ok(prepared) => PreparedGraphics::Frame(prepared),
                Err((command, error)) => PreparedGraphics::Command(ProcessedCommand::Error {
                    command: Some(*command),
                    error,
                }),
            },
        }
    }
}
