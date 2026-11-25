//! The Quality column's tooltip and sort, renaming a genre, composer or year,
//! and the EQ preset match, against Python's answers
//! (`rust/tools/gen_player_vectors.py` → `python_library.json`).

use realhires::eq::EqState;
use realhires::json::{self, Value};
use realhires::musiclib::{self, Track};
use realhires::playerstate::{EqPreset, PlayerState};
use realhires::tagedit;

fn load(name: &str) -> Value {
    let path = format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn pairs(v: &Value) -> Vec<(String, String)> {
    match v {
        Value::Object(o) => o.iter().map(|(k, x)| (k.clone(), text(x))).collect(),
        _ => vec![],
    }
}

#[test]
fn quality_column_matches_python() {
    let data = load("python_library.json");
    for case in data.get("quality").unwrap().as_array().unwrap() {
        let t = Track::from_json(case.get("track").unwrap()).unwrap();
        assert_eq!(musiclib::quality(&t), text(case.get("quality").unwrap()), "{}", t.path);
        assert_eq!(musiclib::quality_detail(&t), text(case.get("detail").unwrap()), "{}", t.path);
        let sort = case.get("sort").unwrap().as_array().unwrap();
        let (bits, rate, float, kbps) = musiclib::quality_sort(&t);
        assert_eq!(
            (bits, rate, float, kbps),
            (sort[0].as_i64().unwrap(), sort[1].as_i64().unwrap(), sort[2].as_bool().unwrap(), sort[3].as_i64().unwrap()),
            "{}",
            t.path
        );
    }
}

#[test]
fn renaming_groups_matches_python() {
    let data = load("python_library.json");
    for d in data.get("dates").unwrap().as_array().unwrap() {
        let d = d.as_array().unwrap();
        assert_eq!(tagedit::renamed_date(&text(&d[0]), &text(&d[1])), text(&d[2]), "{:?}", d[0]);
    }
    for c in data.get("group_names").unwrap().as_array().unwrap() {
        let got = tagedit::check_group_name(&text(c.get("kind").unwrap()), &text(c.get("name").unwrap()));
        match (c.get("ok"), got) {
            (Some(ok), Ok(n)) => assert_eq!(n, text(ok)),
            (None, Err(e)) => assert_eq!(e.0, text(c.get("error").unwrap())),
            (want, got) => panic!("{:?}: wanted {want:?}, got {got:?}", c.get("name")),
        }
    }
    for c in data.get("group_changes").unwrap().as_array().unwrap() {
        let got = tagedit::group_changes(&text(c.get("kind").unwrap()), &text(c.get("name").unwrap()), &pairs(c.get("tags").unwrap()));
        assert_eq!(got, pairs(c.get("changes").unwrap()));
    }
}

#[test]
fn preset_match_ignores_the_on_switch_as_in_python() {
    let data = load("python_library.json");
    let mut st = PlayerState::default();
    for p in data.get("presets").unwrap().as_array().unwrap() {
        st.eq_presets.push(EqPreset { name: text(p.get("name").unwrap()), eq: EqState::from_json(p.get("eq").unwrap()) });
    }
    for m in data.get("matches").unwrap().as_array().unwrap() {
        st.eq = EqState::from_json(m.get("eq").unwrap());
        assert_eq!(st.matching_eq_preset(), text(m.get("match").unwrap()));
    }
}
