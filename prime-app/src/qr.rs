//! The animated-QR transport. Requests arrive as `fps-request` codes the system scanner has already
//! reassembled; answers leave as looping `fps-response` frames. The framing itself lives in
//! `fps_transport::ur`, shared with the phone side.

use fps_transport::ur::{self, UR_TYPE_REQUEST, UR_TYPE_RESPONSE};
use slint_keyos_platform::slint::{ModelRc, SharedString, VecModel};

/// The request in a scanned code, if it is one.
pub fn request_payload(ur_type: &str, data: Vec<u8>) -> Result<Vec<u8>, String> {
    ur::scanned_message(UR_TYPE_REQUEST, ur_type, data).map_err(|error| error.to_string())
}

fn model(frames: Vec<String>) -> ModelRc<SharedString> {
    let frames: Vec<SharedString> = frames.into_iter().map(SharedString::from).collect();
    ModelRc::new(VecModel::from(frames))
}

/// The frames of an answer for the phone.
pub fn response_frames(reply: &[u8]) -> ModelRc<SharedString> {
    model(ur::encode(UR_TYPE_RESPONSE, reply))
}

/// One static code holding `text`, such as an ecash token.
pub fn static_frame(text: &str) -> ModelRc<SharedString> {
    model(vec![text.to_owned()])
}
