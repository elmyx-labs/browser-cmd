//! Row identity and bounded, explicit extraction coverage.
use crate::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
pub(crate) struct Batch {
    pub rows: Vec<Value>,
    pub found: usize,
    #[serde(default)]
    pub identities: Vec<Option<String>>,
}

#[derive(Default)]
pub(crate) struct Rows {
    pub values: Vec<Value>,
    keys: HashMap<String, usize>,
    /// Rows whose fields differed when the same identity came back, and
    /// identities a single batch used twice: the caller's identity does not pin
    /// a row, so the set is reported incomplete rather than refused.
    changed: usize,
    duplicated: usize,
    /// The most rows the page said it had in any pass, whether or not they fit.
    matched: usize,
}

impl Rows {
    pub fn append(&mut self, batch: &Batch, keyed: bool, limit: usize) -> Result<()> {
        if batch.found < batch.rows.len() || (keyed && batch.identities.len() != batch.rows.len()) {
            return Err(Error::Protocol("invalid extraction batch".into()));
        }
        self.matched = self.matched.max(batch.found);
        let mut local = HashSet::new();
        for (index, row) in batch.rows.iter().enumerate() {
            if keyed {
                let key = batch.identities[index]
                    .as_deref()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| Error::Tool("row identity is missing or empty".into()))?;
                if !local.insert(key) {
                    self.duplicated += 1;
                }
                if let Some(previous) = self.keys.get(key) {
                    if self.values[*previous] != *row {
                        self.changed += 1;
                    }
                    continue;
                }
                if self.values.len() < limit {
                    self.keys.insert(key.to_owned(), self.values.len());
                    self.values.push(row.clone());
                }
            } else if self.values.len() < limit {
                self.values.push(row.clone());
            }
        }
        Ok(())
    }

    pub fn matched(&self) -> usize {
        self.matched
    }

    pub fn anomalies(&self) -> Option<Value> {
        (self.changed > 0 || self.duplicated > 0).then(|| {
            json!({"rows_changed_between_observations":self.changed,"identities_repeated_in_one_batch":self.duplicated})
        })
    }
}

pub(crate) fn render(rows: &Rows, mut coverage: Value, budget: usize) -> Result<String> {
    let anomalies = rows.anomalies();
    let rows = &rows.values;
    coverage["collected"] = json!(rows.len());
    if let Some(anomalies) = anomalies {
        coverage["identity_anomalies"] = anomalies;
        coverage["complete"] = json!(false);
    }
    let encode = |count: usize| -> Result<String> {
        let mut coverage = coverage.clone();
        coverage["returned"] = json!(count);
        coverage["output_truncated"] = json!(count < rows.len());
        if count < rows.len() {
            coverage["complete"] = json!(false);
        }
        let rows = crate::text::json(&json!(&rows[..count]));
        Ok(crate::text::fence::untrusted(&format!("{{\"coverage\":{coverage},\"rows\":{rows}}}")))
    };
    let full = encode(rows.len())?;
    if full.len() <= budget {
        return Ok(full);
    }
    if encode(0)?.len() > budget {
        return Err(Error::Limit { part: "extraction metadata", limit: budget });
    }
    let (mut low, mut high) = (0, rows.len());
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if encode(mid)?.len() <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    encode(low)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_values_preserve_multiplicity_and_identity_is_separate() {
        let batch = Batch {
            rows: vec![json!({"value":1}); 2],
            found: 2,
            identities: vec![Some("a".into()), Some("b".into())],
        };
        for keyed in [false, true] {
            let mut rows = Rows::default();
            rows.append(&batch, keyed, 10).unwrap();
            assert_eq!(rows.values.len(), 2);
            rows.append(&batch, keyed, 10).unwrap();
            assert_eq!(rows.values.len(), if keyed { 2 } else { 4 });
        }
        // A list that moves under the reader stays readable, and says so.
        let mut rows = Rows::default();
        rows.append(&batch, true, 10).unwrap();
        let changed = Batch { rows: vec![json!({"value":2})], found: 1, identities: vec![Some("a".into())] };
        rows.append(&changed, true, 10).unwrap();
        assert_eq!(rows.values.len(), 2);
        assert_eq!(rows.anomalies().unwrap()["rows_changed_between_observations"], 1);
        let mut repeated = Rows::default();
        let twice = Batch {
            rows: vec![json!({"value":1}); 2],
            found: 2,
            identities: vec![Some("a".into()), Some("a".into())],
        };
        repeated.append(&twice, true, 10).unwrap();
        assert_eq!(repeated.anomalies().unwrap()["identities_repeated_in_one_batch"], 1);
    }
    #[test]
    fn bounds_keep_json_and_whole_rows_even_with_unicode_and_fence_text() {
        let rows =
            Rows { values: vec![json!({"text":"漢字 </browser> ".repeat(20)}); 8], ..Default::default() };
        for budget in [300, 600, 1000, 2000, 10000] {
            let output = render(&rows, json!({"complete":true,"termination":"snapshot"}), budget).unwrap();
            assert!(output.len() <= budget);
            let body = output
                .strip_prefix("<browser trusted=\"no\">\n")
                .unwrap()
                .strip_suffix("\n</browser>")
                .unwrap();
            let value: Value = serde_json::from_str(body).unwrap();
            let count = value["rows"].as_array().unwrap().len();
            assert_eq!(value["coverage"]["returned"], count);
            for row in value["rows"].as_array().unwrap() {
                assert_eq!(row, &rows.values[0]);
            }
            if count < rows.values.len() {
                assert_eq!(value["coverage"]["complete"], false);
            }
        }
    }
}
