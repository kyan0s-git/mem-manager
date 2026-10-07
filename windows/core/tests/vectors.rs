//! Cross-implementation conformance: every vector in `spec/test-vectors/`
//! must be reproduced exactly (decisions) and within 1e-9 (numbers).

use memmanager_core::leak::analyze;
use memmanager_core::profile::Profile;
use memmanager_core::state::StateMachine;
use serde_json::Value;
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/test-vectors")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn load(prefix: &str) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = std::fs::read_dir(vectors_dir())
        .expect("spec/test-vectors exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".json"))
        })
        .map(|p| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (p.file_name().unwrap().to_string_lossy().into_owned(), v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no {prefix}* vectors found");
    out
}

#[test]
fn leak_vectors() {
    for (name, v) in load("leak-") {
        let profile = Profile::parse(v["profile"].as_str().unwrap()).unwrap();
        let heavy = v["heavy"].as_bool().unwrap();
        let series: Vec<(f64, f64)> = v["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
            .collect();
        let got = analyze(&series, profile.leak_thresholds(), heavy, true);
        let e = &v["expect"];
        assert_eq!(
            got.eligible,
            e["eligible"].as_bool().unwrap(),
            "{name}: eligible"
        );
        assert_eq!(
            got.plateau,
            e["plateau"].as_bool().unwrap(),
            "{name}: plateau"
        );
        assert_eq!(got.leak, e["leak"].as_bool().unwrap(), "{name}: leak");
        assert_eq!(
            got.runaway,
            e["runaway"].as_bool().unwrap(),
            "{name}: runaway"
        );
        for (k, g) in [("slope", got.slope), ("tau", got.tau), ("z", got.z)] {
            let want = e[k].as_f64().unwrap();
            assert!(close(g, want), "{name}: {k} got {g} want {want}");
        }
    }
}

#[test]
fn state_vectors() {
    for (name, v) in load("state-") {
        let mut sm = StateMachine::new();
        let got: Vec<&str> = v["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let a = s.as_array().unwrap();
                let event = a.get(2).and_then(|e| e.as_bool()).unwrap_or(false);
                sm.step(a[0].as_u64().unwrap(), a[1].as_f64().unwrap(), event)
                    .name()
            })
            .collect();
        let want: Vec<&str> = v["expect_states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert_eq!(got, want, "{name}");
    }
}
