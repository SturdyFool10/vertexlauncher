use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct LruEntry<V> {
    pub value: V,
    pub approx_bytes: usize,
    pub last_used_tick: u64,
}

#[derive(Debug)]
pub struct LruState<K, V> {
    entries: HashMap<K, LruEntry<V>>,
    max_bytes: usize,
    total_bytes: usize,
    tick: u64,
}

impl<K, V> LruState<K, V>
where
    K: Clone + Eq + Hash,
{
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_bytes,
            total_bytes: 0,
            tick: 0,
        }
    }

    pub fn set_max_bytes(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &K) -> Option<&LruEntry<V>> {
        self.entries.get(key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut LruEntry<V>> {
        self.entries.get_mut(key)
    }

    pub fn touch(&mut self, key: &K) -> Option<&mut LruEntry<V>> {
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        entry.last_used_tick = tick;
        Some(entry)
    }

    pub fn insert(&mut self, key: K, value: V, approx_bytes: usize) -> Vec<(K, V)> {
        self.tick = self.tick.saturating_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.approx_bytes);
        }
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.entries.insert(
            key,
            LruEntry {
                value,
                approx_bytes,
                last_used_tick: self.tick,
            },
        );
        self.evict_to_budget()
    }

    pub fn insert_without_eviction(&mut self, key: K, value: V, approx_bytes: usize) {
        self.tick = self.tick.saturating_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(previous.approx_bytes);
        }
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.entries.insert(
            key,
            LruEntry {
                value,
                approx_bytes,
                last_used_tick: self.tick,
            },
        );
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let entry = self.entries.remove(key)?;
        self.total_bytes = self.total_bytes.saturating_sub(entry.approx_bytes);
        Some(entry.value)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut LruEntry<V>) -> bool) -> Vec<(K, V)> {
        let mut keys_to_remove = Vec::new();
        for (key, entry) in self.entries.iter_mut() {
            if !keep(key, entry) {
                keys_to_remove.push(key.clone());
            }
        }

        let mut evicted = Vec::with_capacity(keys_to_remove.len());
        for key in keys_to_remove {
            if let Some(entry) = self.entries.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.approx_bytes);
                evicted.push((key, entry.value));
            }
        }
        evicted
    }

    pub fn evict_to_budget(&mut self) -> Vec<(K, V)> {
        self.evict_to_budget_where(|_, _| true)
    }

    pub fn evict_to_budget_where(
        &mut self,
        mut can_evict: impl FnMut(&K, &LruEntry<V>) -> bool,
    ) -> Vec<(K, V)> {
        if self.total_bytes <= self.max_bytes {
            return Vec::new();
        }

        // Collect eligible keys sorted oldest-first in one pass.
        let mut eviction_order: Vec<(K, u64)> = self
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                can_evict(key, entry).then_some((key.clone(), entry.last_used_tick))
            })
            .collect();
        eviction_order.sort_unstable_by_key(|(_, tick)| *tick);

        let mut evicted = Vec::new();
        for (key, _) in eviction_order {
            if self.total_bytes <= self.max_bytes {
                break;
            }
            if let Some(entry) = self.entries.remove(&key) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.approx_bytes);
                evicted.push((key, entry.value));
            }
        }
        evicted
    }

    pub fn pop_lru(&mut self) -> Option<(K, V)> {
        self.pop_lru_where(|_, _| true)
    }

    pub fn pop_lru_where(
        &mut self,
        mut can_evict: impl FnMut(&K, &LruEntry<V>) -> bool,
    ) -> Option<(K, V)> {
        let key = self
            .entries
            .iter()
            .filter(|(key, entry)| can_evict(key, entry))
            .min_by_key(|(_, entry)| entry.last_used_tick)
            .map(|(key, _)| key.clone())?;
        let entry = self.entries.remove(&key)?;
        self.total_bytes = self.total_bytes.saturating_sub(entry.approx_bytes);
        Some((key, entry.value))
    }

    pub fn clear(&mut self) -> Vec<(K, V)> {
        self.total_bytes = 0;
        self.entries
            .drain()
            .map(|(key, entry)| (key, entry.value))
            .collect()
    }

    pub fn values_any(&self, mut pred: impl FnMut(&LruEntry<V>) -> bool) -> bool {
        self.entries.values().any(|e| pred(e))
    }

    /// Look up an entry by any type that the stored key can borrow as.
    /// This avoids an allocation when `K = String` and the caller has a `&str`.
    pub fn get_borrowed<Q>(&self, key: &Q) -> Option<&LruEntry<V>>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        self.entries.get(key)
    }

    pub fn keys_cloned(&self) -> Vec<K> {
        self.entries.keys().cloned().collect()
    }

    pub fn values_cloned(&self) -> Vec<V>
    where
        V: Clone,
    {
        self.entries
            .values()
            .map(|entry| entry.value.clone())
            .collect()
    }

    pub fn entries_cloned(&self) -> Vec<(K, LruEntry<V>)>
    where
        V: Clone,
    {
        self.entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct ThreadSafeLru<K, V> {
    inner: Arc<Mutex<LruState<K, V>>>,
}

impl<K, V> ThreadSafeLru<K, V>
where
    K: Clone + Eq + Hash,
{
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LruState::new(max_bytes))),
        }
    }

    pub fn read<R>(&self, f: impl FnOnce(&LruState<K, V>) -> R) -> R {
        let state = self.inner.lock().expect("thread-safe lru mutex poisoned");
        f(&state)
    }

    pub fn write<R>(&self, f: impl FnOnce(&mut LruState<K, V>) -> R) -> R {
        let mut state = self.inner.lock().expect("thread-safe lru mutex poisoned");
        f(&mut state)
    }
}
