//! Animated QR codes between the phone and Prime.

use std::sync::{Arc, Mutex, PoisonError};

use fps_transport::ur::{self, PartDecoder, UR_TYPE_REQUEST, UR_TYPE_RESPONSE};

/// The QR codes that carry `request` to Prime, to show one after another in a loop. Prime's
/// scanner collects them in any order.
#[uniffi::export]
pub fn request_frames(request: Vec<u8>) -> Vec<String> {
    ur::encode(UR_TYPE_REQUEST, &request)
}

/// Where reading Prime's answer stands after one more code.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Scan {
    /// More codes are needed; roughly this much has arrived, from 0 to 1.
    Partial { progress: f64 },
    /// The whole answer.
    Complete { answer: Vec<u8> },
    /// The code is not part of an answer from Prime, or belongs to another answer.
    Rejected { reason: String },
}

/// Collects the animated QR codes of one answer from Prime as the camera reads them.
#[derive(uniffi::Object)]
pub struct AnswerScanner {
    decoder: Mutex<PartDecoder>,
}

impl Default for AnswerScanner {
    fn default() -> Self {
        Self {
            decoder: Mutex::new(PartDecoder::new(UR_TYPE_RESPONSE)),
        }
    }
}

#[uniffi::export]
impl AnswerScanner {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Takes one code the camera read.
    pub fn receive(&self, code: String) -> Scan {
        let mut decoder = self.decoder.lock().unwrap_or_else(PoisonError::into_inner);
        match decoder.receive(&code) {
            Ok(Some(answer)) => Scan::Complete { answer },
            Ok(None) => Scan::Partial {
                progress: decoder.progress(),
            },
            Err(error) => Scan::Rejected {
                reason: error.to_string(),
            },
        }
    }
}
