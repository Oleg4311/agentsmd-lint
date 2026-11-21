//! Сверка собственного разборщика JSON с serde_json: на случайных документах и их порче принятие и результат должны совпадать.
use agentsmd_lint::json::{self, Json};
use proptest::prelude::*;

fn same(a: &Json, b: &serde_json::Value) -> bool {
    match (a, b) {
        (Json::Null, serde_json::Value::Null) => true,
        (Json::Bool(x), serde_json::Value::Bool(y)) => x == y,
        (Json::Num(x), serde_json::Value::Number(y)) => y
            .as_f64()
            .is_some_and(|y| (x - y).abs() <= y.abs() * 1e-12 || x == &y),
        (Json::Str(x), serde_json::Value::String(y)) => x == y,
        (Json::Arr(x), serde_json::Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        (Json::Obj(x), serde_json::Value::Object(y)) => {
            // serde_json::Map без порядка и с последним значением для повторов ключей
            let mut keys: Vec<&String> = x.iter().map(|(k, _)| k).collect();
            keys.sort();
            keys.dedup();
            keys.len() == y.len()
                && keys
                    .iter()
                    .all(|k| a.get(k).zip(y.get(*k)).is_some_and(|(a, b)| same(a, b)))
        }
        _ => false,
    }
}

fn arb_json() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("null".to_string()),
        Just("true".to_string()),
        Just("false".to_string()),
        (-1000i64..1000).prop_map(|n| n.to_string()),
        (-1e6f64..1e6).prop_map(|f| format!("{f}")),
        "[a-zA-Z0-9 _\\-\\\\\"/é😀]{0,12}".prop_map(|s| format!("\"{}\"", json::escape(&s))),
    ];
    leaf.prop_recursive(5, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(|v| format!("[{}]", v.join(","))),
            prop::collection::vec(("[a-z]{1,4}", inner), 0..5).prop_map(|v| format!(
                "{{{}}}",
                v.into_iter()
                    .map(|(k, x)| format!("\"{k}\":{x}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, ..ProptestConfig::default() })]

    #[test]
    fn valid_documents_parse_identically(doc in arb_json(), pad in "[ \\t\\n\\r]{0,3}") {
        let text = format!("{pad}{doc}{pad}");
        let mine = json::parse(&text).expect("собственный разборщик принял валидный документ");
        let theirs: serde_json::Value = serde_json::from_str(&text).unwrap();
        prop_assert!(same(&mine, &theirs), "{text}");
    }

    #[test]
    fn corrupted_documents_are_accepted_or_rejected_alike(doc in arb_json(), cut in 0usize..200, junk in prop::sample::select(vec!["", ",", "}", "]", "\"", "x", "1", " ", "\\", "{", "[", ":", "-", "."])) {
        let chars: Vec<char> = doc.chars().collect();
        let at = cut % (chars.len() + 1);
        let mut text: String = chars[..at].iter().collect();
        text.push_str(junk);
        text.extend(chars[at..].iter().skip(usize::from(cut % 3 == 0)));
        let mine = json::parse(&text);
        let theirs = serde_json::from_str::<serde_json::Value>(&text);
        prop_assert_eq!(mine.is_ok(), theirs.is_ok(), "{}", text);
        if let (Ok(a), Ok(b)) = (&mine, &theirs) {
            prop_assert!(same(a, b), "{}", text);
        }
    }

    #[test]
    fn arbitrary_text_never_panics_and_agrees(text in "\\PC{0,40}") {
        let mine = json::parse(&text).is_ok();
        let theirs = serde_json::from_str::<serde_json::Value>(&text).is_ok();
        prop_assert_eq!(mine, theirs, "{}", text);
    }
}

#[test]
fn deep_nesting_both_reject_at_the_same_limit() {
    for depth in [100usize, 127, 128, 129, 200] {
        let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let mine = json::parse(&text).is_ok();
        let theirs = serde_json::from_str::<serde_json::Value>(&text).is_ok();
        assert_eq!(mine, theirs, "глубина {depth}");
    }
}
