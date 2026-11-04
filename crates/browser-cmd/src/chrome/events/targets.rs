//! The browser's pages as its Target events and target lists tell them:
//! where each is, what opened it, whether it crashed.

use serde_json::Value;

#[derive(Default)]
pub(crate) struct Targets {
    /// Each page's TargetInfo as last told, `crashed` set when its renderer died.
    pages: Vec<Value>,
}

fn id(page: &Value) -> &str {
    page["targetId"].as_str().unwrap_or_default()
}

impl Targets {
    pub(crate) fn event(&mut self, method: &str, params: &Value) {
        match method {
            "Target.targetCreated" | "Target.targetInfoChanged" => self.told(&params["targetInfo"]),
            "Target.targetDestroyed" => {
                let gone = params["targetId"].as_str().unwrap_or_default();
                self.pages.retain(|page| id(page) != gone);
            }
            "Target.targetCrashed" => {
                let crashed = params["targetId"].as_str().unwrap_or_default();
                if let Some(page) = self.pages.iter_mut().find(|page| id(page) == crashed) {
                    page["crashed"] = true.into();
                }
            }
            _ => {}
        }
    }

    fn told(&mut self, info: &Value) {
        let target = id(info).to_owned();
        let at = self.pages.iter().position(|page| id(page) == target);
        if info["type"] != "page" || target.is_empty() {
            self.pages.retain(|page| id(page) != target);
            return;
        }
        let mut info = info.clone();
        match at {
            Some(at) => {
                info["crashed"] = self.pages[at]["crashed"].clone();
                self.pages[at] = info;
            }
            None => self.pages.push(info),
        }
    }

    /// The browser's own list replaces what events told, keeping what crashed.
    pub(crate) fn seed(&mut self, list: &[Value]) {
        let old = std::mem::take(&mut self.pages);
        self.pages = list.to_vec();
        for page in &mut self.pages {
            if old.iter().any(|was| id(was) == id(page) && was["crashed"] == true) {
                page["crashed"] = true.into();
            }
        }
    }

    pub(crate) fn pages(&self) -> &[Value] {
        &self.pages
    }

    pub(crate) fn recovered(&mut self, target: &str) {
        if let Some(page) = self.pages.iter_mut().find(|page| id(page) == target) {
            page["crashed"] = false.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pages_are_told_by_events_and_a_crash_is_kept_until_recovered() {
        let mut targets = Targets::default();
        let page = |id: &str, opener: Option<&str>| json!({"targetInfo":{"targetId":id,"type":"page","url":"about:blank","openerId":opener}});
        for (id, opener) in [("A", None), ("B", Some("A")), ("C", Some("B")), ("U", None), ("V", Some("U"))] {
            targets.event("Target.targetCreated", &page(id, opener));
        }
        targets
            .event("Target.targetCreated", &json!({"targetInfo":{"targetId":"W","type":"service_worker"}}));
        assert_eq!(targets.pages().len(), 5);
        targets.event("Target.targetCrashed", &json!({"targetId":"B"}));
        targets.event("Target.targetInfoChanged", &page("B", Some("A")));
        let listed = targets.pages().to_vec();
        targets.seed(&listed);
        assert_eq!(targets.pages().iter().filter(|page| page["crashed"] == true).count(), 1);
        targets.recovered("B");
        assert!(targets.pages().iter().all(|page| page["crashed"] != true));
        targets.event("Target.targetDestroyed", &json!({"targetId":"C"}));
        assert_eq!(targets.pages().len(), 4);
    }
}
