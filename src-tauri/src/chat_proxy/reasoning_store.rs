use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

/// 有界的 reasoning_content 存储。冲突的调用 ID 不得跨会话复用内容。
#[derive(Default)]
pub(super) struct ReasoningStore {
    entries: HashMap<String, Arc<str>>,
    order: VecDeque<String>,
    call_alias_ids: HashSet<String>,
    ambiguous_call_ids: HashSet<String>,
    reject_all_call_aliases: bool,
}

impl ReasoningStore {
    const MAX_ENTRIES: usize = 2000;
    const MAX_AMBIGUOUS_CALL_IDS: usize = Self::MAX_ENTRIES;
    const MAX_CONTENT_BYTES: usize = 1024 * 1024;

    #[cfg(test)]
    pub(super) fn insert(&mut self, id: &str, content: &str) {
        if content.trim().is_empty() {
            return;
        }
        self.insert_shared(id, Self::bounded_content(content));
    }

    pub(super) fn bounded_content(content: &str) -> Arc<str> {
        if content.len() <= Self::MAX_CONTENT_BYTES {
            Arc::<str>::from(content)
        } else {
            let mut end = Self::MAX_CONTENT_BYTES;
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            Arc::<str>::from(&content[..end])
        }
    }

    pub(super) fn insert_shared(&mut self, id: &str, content: Arc<str>) {
        if self.entries.contains_key(id) {
            self.entries.insert(id.to_owned(), content);
            return;
        }
        while self.entries.len() >= Self::MAX_ENTRIES {
            let Some(oldest) = self.order.pop_front() else {
                self.entries.clear();
                break;
            };
            if self.entries.remove(&oldest).is_some() {
                if self.call_alias_ids.remove(&oldest) {
                    self.mark_ambiguous(oldest);
                }
                break;
            }
        }
        let owned_id = id.to_owned();
        self.entries.insert(owned_id.clone(), content);
        self.order.push_back(owned_id);
    }

    pub(super) fn insert_call_alias(&mut self, call_id: &str, content: Arc<str>) {
        if self.reject_all_call_aliases || self.ambiguous_call_ids.contains(call_id) {
            return;
        }
        if self.entries.contains_key(call_id) {
            self.entries.remove(call_id);
            self.call_alias_ids.remove(call_id);
            self.mark_ambiguous(call_id.to_owned());
            return;
        }
        self.insert_shared(call_id, content);
        if self.reject_all_call_aliases {
            self.entries.remove(call_id);
            return;
        }
        self.call_alias_ids.insert(call_id.to_owned());
    }

    pub(super) fn get(&self, id: &str) -> Option<&str> {
        self.entries.get(id).map(AsRef::as_ref)
    }

    pub(super) fn subset<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Self {
        let entries = ids
            .into_iter()
            .filter_map(|id| {
                self.entries
                    .get(id)
                    .map(|content| (id.to_owned(), content.clone()))
            })
            .collect();
        Self {
            entries,
            ..Self::default()
        }
    }

    fn mark_ambiguous(&mut self, id: String) {
        if self.ambiguous_call_ids.len() >= Self::MAX_AMBIGUOUS_CALL_IDS {
            self.entries.clear();
            self.order.clear();
            self.call_alias_ids.clear();
            self.ambiguous_call_ids.clear();
            self.reject_all_call_aliases = true;
            return;
        }
        self.ambiguous_call_ids.insert(id);
    }
}

#[cfg(test)]
mod tests {
    use super::ReasoningStore;
    use std::sync::Arc;

    #[test]
    fn stays_bounded_under_ambiguous_call_ids() {
        let mut store = ReasoningStore::default();
        for index in 0..ReasoningStore::MAX_ENTRIES * 4 {
            let id = format!("call_{index}");
            store.insert_call_alias(&id, Arc::from("conversation A"));
            store.insert_call_alias(&id, Arc::from("conversation B"));
        }

        assert!(store.entries.len() <= ReasoningStore::MAX_ENTRIES);
        assert!(store.order.len() <= ReasoningStore::MAX_ENTRIES);
        assert!(store.call_alias_ids.len() <= ReasoningStore::MAX_ENTRIES);
        assert!(store.ambiguous_call_ids.len() <= ReasoningStore::MAX_ENTRIES);
        assert!(store.reject_all_call_aliases);

        store.insert_call_alias("call_0", Arc::from("conversation C"));
        store.insert_call_alias("new_call", Arc::from("conversation D"));
        assert_eq!(store.get("call_0"), None);
        assert_eq!(store.get("new_call"), None);
    }

    #[test]
    fn repeated_call_id_is_ambiguous_even_when_content_matches() {
        let mut store = ReasoningStore::default();
        store.insert_call_alias("call_1", Arc::from("same content"));
        store.insert_call_alias("call_1", Arc::from("same content"));

        assert_eq!(store.get("call_1"), None);
    }

    #[test]
    fn evicted_call_id_cannot_be_reused() {
        let mut store = ReasoningStore::default();
        store.insert_call_alias("call_1", Arc::from("conversation A"));
        for index in 0..ReasoningStore::MAX_ENTRIES {
            store.insert_shared(&format!("msg_{index}"), Arc::from("other reasoning"));
        }

        store.insert_call_alias("call_1", Arc::from("conversation B"));

        assert_eq!(store.get("call_1"), None);
    }

    #[test]
    fn alias_that_triggers_fail_closed_is_not_retained() {
        let mut store = ReasoningStore::default();
        store.ambiguous_call_ids.extend(
            (0..ReasoningStore::MAX_AMBIGUOUS_CALL_IDS).map(|index| format!("ambiguous_{index}")),
        );
        store.insert_call_alias("old_call", Arc::from("old reasoning"));
        for index in 0..ReasoningStore::MAX_ENTRIES - 1 {
            store.insert_shared(&format!("msg_{index}"), Arc::from("other reasoning"));
        }

        store.insert_call_alias("trigger_call", Arc::from("private reasoning"));

        assert!(store.reject_all_call_aliases);
        assert_eq!(store.get("trigger_call"), None);
    }
}
