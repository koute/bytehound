use std::{
    collections::{
        hash_map::{Entry, RandomState},
        HashMap,
    },
    hash::{BuildHasher, Hash},
    marker::PhantomData,
};

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
struct Item<K, V> {
    prev: Option<K>,
    next: Option<K>,
    value: V,
}

#[derive(Clone, Debug)]
pub struct OrderedMap<K, V, S = RandomState>
where
    K: Copy + PartialEq + Eq + Hash,
{
    first_and_last: Option<(K, K)>,
    map: HashMap<K, Item<K, V>, S>,
    phantom: PhantomData<S>,
}

impl<K, V, S> Default for OrderedMap<K, V, S>
where
    K: Copy + PartialEq + Eq + Hash,
    S: BuildHasher + Default,
{
    fn default() -> Self {
        OrderedMap {
            first_and_last: None,
            map: Default::default(),
            phantom: PhantomData,
        }
    }
}

impl<K, V> OrderedMap<K, V, RandomState>
where
    K: Copy + PartialEq + Eq + Hash,
{
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<K, V, S> OrderedMap<K, V, S>
where
    K: Copy + PartialEq + Eq + Hash,
    S: BuildHasher,
{
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        debug_assert_eq!(self.first_and_last.is_none(), self.map.is_empty());
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|item| &item.value)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.map.get_mut(key).map(|item| &mut item.value)
    }

    pub fn front_key(&self) -> Option<K> {
        self.first_and_last.map(|(key, _)| key)
    }

    #[allow(dead_code)]
    pub fn back_key(&self) -> Option<K> {
        self.first_and_last.map(|(_, key)| key)
    }

    pub fn pop_front(&mut self) -> Option<(K, V)> {
        let key = self.front_key()?;
        self.remove(&key).map(|value| (key, value))
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.map.entry(key) {
            Entry::Occupied(mut bucket) => {
                let (first, last) = self.first_and_last.unwrap();

                let bucket = bucket.get_mut();
                let old_value = std::mem::replace(&mut bucket.value, value);

                if key != last {
                    let old_next = bucket.next.take().unwrap();
                    let old_prev = bucket.prev.take();

                    if key == first {
                        bucket.prev = Some(last);
                        self.first_and_last = Some((old_next, key));
                    } else {
                        let old_prev = old_prev.unwrap();
                        bucket.prev = Some(last);
                        self.first_and_last = Some((first, key));
                        self.map.get_mut(&old_prev).unwrap().next = Some(old_next);
                    }

                    self.map.get_mut(&old_next).unwrap().prev = old_prev;
                    self.map.get_mut(&last).unwrap().next = Some(key);
                }

                Some(old_value)
            }
            Entry::Vacant(bucket) => {
                if let Some((first, last)) = self.first_and_last {
                    self.first_and_last = Some((first, key));
                    bucket.insert(Item {
                        prev: Some(last),
                        next: None,
                        value,
                    });

                    self.map.get_mut(&last).unwrap().next = Some(key);
                } else {
                    self.first_and_last = Some((key, key));
                    bucket.insert(Item {
                        prev: None,
                        next: None,
                        value,
                    });
                }

                None
            }
        }
    }

    #[allow(dead_code)]
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let key = *key;
        if let Some(item) = self.map.remove(&key) {
            if self.map.is_empty() {
                self.first_and_last = None;
                return Some(item.value);
            }

            if let Some(next) = item.next {
                self.map.get_mut(&next).unwrap().prev = item.prev;
            }
            if let Some(prev) = item.prev {
                self.map.get_mut(&prev).unwrap().next = item.next;
            }

            let (mut first, mut last) = self.first_and_last.unwrap();
            if first == key {
                first = item.next.unwrap();
            }
            if last == key {
                last = item.prev.unwrap();
            }
            self.first_and_last = Some((first, last));

            Some(item.value)
        } else {
            None
        }
    }
}

#[test]
fn test_ordered_copy_map() {
    macro_rules! check {
        ($map:expr, $expected:expr) => {
            let expected: Vec<i32> = $expected.iter().copied().collect();
            let mut actual_front = Vec::new();
            let mut actual_back = Vec::new();

            {
                let mut map = $map.clone();
                while let Some(key) = map.front_key() {
                    map.remove(&key);
                    actual_front.push(key);
                }
            }
            {
                let mut map = $map.clone();
                while let Some(key) = map.back_key() {
                    map.remove(&key);
                    actual_back.push(key);
                }
            }

            assert_eq!(actual_front, expected);
            assert_eq!($map.len(), expected.len());
            actual_back.reverse();
            assert_eq!(actual_front, actual_back);
        };
    }

    macro_rules! insert {
        ($map:expr, $key:expr) => {
            $map.insert($key, ());
        };
    }

    let mut map = OrderedMap::new();
    check!(map, &[]);

    insert!(map, 10);
    check!(map, &[10]);

    insert!(map, 5);
    check!(map, &[10, 5]);

    insert!(map, 15);
    check!(map, &[10, 5, 15]);

    insert!(map, 15); // Insert one that's already at the end.
    check!(map, &[10, 5, 15]);

    insert!(map, 5); // Insert one from the middle.
    check!(map, &[10, 15, 5]);

    insert!(map, 10); // Insert one from the start.
    check!(map, &[15, 5, 10]);

    insert!(map, 20);
    check!(map, &[15, 5, 10, 20]);

    map.remove(&15); // Remove first.
    check!(map, &[5, 10, 20]);

    map.remove(&10); // Remove middle.
    check!(map, &[5, 20]);

    map.remove(&20); // Remove last.
    check!(map, &[5]);

    map.remove(&5); // Remove first and last.
    check!(map, &[]);
}
