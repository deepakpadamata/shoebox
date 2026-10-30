//! The CLIP tokenizer against Hugging Face `tokenizers` 0.20.3 with the model's `tokenizer.json`
//! (ids made by a script; `"__LONG__"` stands for 300 × "x").

use shoebox_search::clip_tokenizer::{ClipTokenizer, CONTEXT, END, START};

const CASES: &[(&str, &[i32])] = &[
    (
        "a dog on a beach",
        &[49406, 320, 1929, 525, 320, 2117, 49407],
    ),
    ("Birthday cake!!", &[49406, 1166, 2972, 748, 49407]),
    (
        "it's the 1990s, isn't it?",
        &[
            49406, 585, 568, 518, 272, 280, 280, 271, 338, 267, 2923, 713, 585, 286, 49407,
        ],
    ),
    (
        "São Paulo at night",
        &[49406, 45911, 21628, 536, 930, 49407],
    ),
    (
        "red car 2019",
        &[49406, 736, 1615, 273, 271, 272, 280, 49407],
    ),
    ("hello   world\n\ttab", &[49406, 3306, 1002, 14724, 49407]),
    (
        "!!'s weird...punctuation",
        &[49406, 1443, 262, 338, 5613, 678, 47051, 49407],
    ),
    (
        "naïve café résumé",
        &[49406, 1097, 35689, 563, 15304, 29106, 7054, 4166, 49407],
    ),
    (
        "日本の桜",
        &[49406, 39121, 44353, 21575, 162, 94, 506, 49407],
    ),
    ("emoji 🐶🏖️", &[49406, 16327, 14466, 1109, 28877, 49407]),
    (
        "__LONG__",
        &[
            49406, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035, 32035,
            32035, 32035, 32035, 22819, 49407,
        ],
    ),
    ("", &[49406, 49407]),
    ("rock'n'roll", &[49406, 2172, 262, 333, 262, 3341, 49407]),
    ("don't", &[49406, 847, 713, 49407]),
    (
        "A.B.C. 3.14",
        &[
            49406, 320, 269, 321, 269, 322, 269, 274, 269, 272, 275, 49407,
        ],
    ),
];

fn tokenizer() -> ClipTokenizer {
    ClipTokenizer::from_merges(include_str!("../data/clip-merges.txt")).unwrap()
}

#[test]
fn matches_the_reference_tokenizer() {
    let t = tokenizer();
    let mut failures = vec![];
    for (text, want) in CASES {
        let text = if *text == "__LONG__" {
            "x".repeat(300)
        } else {
            text.to_string()
        };
        let mut got = vec![START];
        got.extend(t.encode(&text));
        got.push(END);
        if got != *want {
            failures.push(format!("{text:?}\n  want {want:?}\n   got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn context_is_cut_and_padded_like_openclip() {
    let t = tokenizer();
    let c = t.context("a dog");
    assert_eq!(&c[..4], &[START, 320, 1929, END]);
    assert!(c[4..].iter().all(|x| *x == 0));
    let long = t.context(&"dog ".repeat(200));
    assert_eq!(long.len(), CONTEXT);
    assert_eq!(long[CONTEXT - 1], END);
}
