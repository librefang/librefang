use librefang_sidecar::Content;
use serde_json::{json, Value};

type PollBuilder = fn(String, Vec<String>, bool, Option<u8>, Option<String>) -> Value;
type PollAnswerBuilder = fn(String, Vec<u8>) -> Value;

#[test]
fn poll_builders_use_kernel_byte_sized_option_ids() {
    let poll: PollBuilder = Content::poll;
    let answer: PollAnswerBuilder = Content::poll_answer;

    assert_eq!(
        poll(
            "Question?".to_string(),
            vec!["A".to_string(), "B".to_string()],
            true,
            Some(u8::MAX),
            None,
        ),
        json!({
            "Poll": {
                "question": "Question?",
                "options": ["A", "B"],
                "is_quiz": true,
                "correct_option_id": 255
            }
        })
    );
    assert_eq!(
        answer("poll-1".to_string(), vec![0, u8::MAX]),
        json!({"PollAnswer": {"poll_id": "poll-1", "option_ids": [0, 255]}})
    );
}
