//! Associative part of the tables

use crate::{
    common::{u32_to_usize, LogSize, ilog2_exact, iexp2},
    load::Error as LoadError,
};

use super::Key;

#[inline]
const fn mask(loglen: LogSize) -> u32 {
    iexp2(Some(loglen)) - 1
}

// https://www.lua.org/source/5.4/lstring.c.html#luaS_hash
const fn str_table_hash_with_seed<const SEED: u32>(value: &str) -> u32 {
    let value = value.as_bytes();
    let mut index = value.len();
    assert!(u32::BITS <= usize::BITS && index <= u32::MAX as usize);
    let mut hash = SEED ^ (index as u32);
    let step = (index >> 5) + 1;
    while index >= step {
        let j = match index.checked_sub(1) {
            Some(j) if j < value.len() => j,
            // SAFETY: i dare you
            _ => unsafe { std::hint::unreachable_unchecked() },
        };
        hash ^= u32::wrapping_add(
            u32::wrapping_add(hash << 5, hash >> 2),
            value[j] as u32 );
        index -= step;
    }
    hash
}

pub(crate) const fn str_table_hash(value: &str) -> u32 {
    str_table_hash_with_seed::<0x_645D_BFCD>(value)
}

// https://www.lua.org/source/5.4/ltable.c.html#hashint
pub(crate) const fn int_table_hash(value: i32, loglen: LogSize) -> u32 {
    if loglen == 0 { return 0; }
    if value >= 0 {
        (value % (mask(loglen) as i32)) as u32
    } else {
        (value as u32) % mask(loglen)
    }
}

pub(crate) type Item<V> = crate::table_iter::AssocItem<Key, V>;

impl Key {
    #[inline]
    fn position(&self, loglen: LogSize) -> u32 {
        match *self {
            Self::Index(index) => int_table_hash(index, loglen),
            Self::Name(ref value) => str_table_hash(value) & mask(loglen),
        }
    }
}

impl<V> Item<V> {
    fn main_position(&self, loglen: LogSize) -> Option<u32> {
        match self {
            Self::Dead { .. } => None,
            Self::Live { key, .. } => Some(key.position(loglen))
        }
    }
    fn relocate(mut self, old_index: u32, new_index: u32) -> Self {
        let link_mut = match &mut self {
            Self::Live { link, .. } | Self::Dead { link } => link,
        };
        if *link_mut != 0 {
            *link_mut += old_index as i32 - new_index as i32;
        }
        self
    }
    fn relocate_link(&mut self, old_index: u32, new_index: u32) {
        let link_mut = match self {
            Self::Live { link, .. } | Self::Dead { link } => link,
        };
        if *link_mut != 0 {
            *link_mut += new_index as i32 - old_index as i32;
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Table<V> {
    // Invariant:
    // if `items` is Some than `items.len()`` is a power of two
    items: Option<Box<[Option<Item<V>>]>>,
    last_free: u32,
}

impl<V> Table<V> {
    pub(super) fn new(loglen: Option<LogSize>) -> Self {
        let size = iexp2(loglen);
        let items = (size > 0).then(|| {
            let mut items = Vec::with_capacity(size as usize);
            items.resize_with(size as usize, || None);
            items
        });
        Self {
            items: items.map(Box::from),
            last_free: size,
        }
    }
    pub(super) fn loglen(&self) -> Option<LogSize> {
        let Ok(loglen) = ilog2_exact(self.items.as_ref()?.len()) else {
            unreachable!("struct invariant");
        };
        loglen
    }
    pub(super) fn len(&self) -> usize {
        if let Some(items) = self.items.as_ref() {
            items.len()
        } else { 0 }
    }
}

impl<V> Table<V> {
    pub fn last_free(&self) -> u32 {
        self.last_free
    }
}

impl<V> Table<V> {
    fn validate_positions<E: LoadError>(&self) -> Result<(), E> {
        let Some(items) = self.items.as_deref() else {
            return Ok(());
        };
        // SAFETY: `items` has a size of a power of two
        let Some(loglen) = self.loglen() else { unreachable!() };
        let len = iexp2(Some(loglen));
        let mut unvalidated: Vec<Option<u32>> = items.iter()
            .enumerate().map( |(index, item)| {
                let index: u32 = index.try_into().unwrap();
                let item = item.as_ref()?;
                Some(item.main_position(loglen).unwrap_or(index))
            }).collect();
        for main_position in 0 .. len {
            let mut position = main_position;
            let mut steps = 0;
            if unvalidated[u32_to_usize(position)] != Some(position) {
                // chain start is not in its main position, which means
                // that no elements can have this position as main position.
                continue;
            }
            loop {
                let index = u32_to_usize(position);
                if unvalidated[index] == Some(main_position) {
                    unvalidated[index] = None;
                }
                let link = match items[index] {
                    Some(Item::Dead { link } | Item::Live { link, .. })
                        if link != 0
                        => link,
                    _ => break,
                };
                let Some(next_position) = position.checked_add_signed(link)
                    .filter(|&pos| pos < len)
                else {
                    return Err(E::from(
                        "assoc node link should lead within bounds" ));
                };
                position = next_position;
                steps += 1;
                if steps >= len {
                    return Err(E::from(
                        "assoc node chain should not form a loop" ));
                }
            }
        }
        for position in 0 .. len {
            let index = u32_to_usize(position);
            if unvalidated[index].is_some() {
                return Err(E::from(
                    "table key should be in a valid position" ));
            }
        }
        Ok(())
    }
}

enum InsertItem<V> {
    Dead { position: u32 },
    Live { key: Key, value: Option<V> },
}

impl<V> InsertItem<V> {
    #[inline]
    fn dead_from_key(key: Key, loglen: LogSize) -> Self {
        Self::Dead { position: key.position(loglen) }
    }
    #[inline]
    fn position(&self, loglen: LogSize) -> u32 {
        match *self {
            Self::Dead { position } => position & mask(loglen),
            Self::Live { ref key, .. } => key.position(loglen),
        }
    }
    #[inline]
    fn into_item(self, link: i32) -> Item<V> {
        match self {
            Self::Dead { .. } => Item::Dead { link },
            Self::Live { key, value } => Item::Live { value, key, link },
        }
    }
}

/// Table builder facilitating conversion from Rust structures
pub(super) struct TableBuilder<V> {
    table: Table<V>,
}

impl<V> TableBuilder<V> {

    pub(super) fn new(loglen: Option<LogSize>) -> Self {
        Self { table: Table::new(loglen) }
    }

    pub(super) fn build(self) -> Table<V> {
        self.table
    }

    pub(super) fn insert(&mut self, key: Key, value: Option<V>) {
        self.insert_item(InsertItem::Live { key, value })
    }

    pub(super) fn insert_dead(&mut self, key: Key) {
        self.insert_item(InsertItem::dead_from_key( key.clone(),
            self.table.loglen().unwrap() ))
    }

    fn insert_item( &mut self,
        item: InsertItem<V>,
    ) {
        let loglen = self.table.loglen()
            .expect("the table should have some space");
        let main_index = item.position(loglen);
        if let free @ &mut None = self.get_mut(main_index) {
            // Lua here would fill dead position as well as free.
            // But we are not Lua: we do not normally make dead positions,
            // and even when we do, we don't want to overwrite them.
            *free = Some(item.into_item(0));
            return;
        }
        let Some(free_index) = self.find_free_index() else {
            panic!("the table should have free space");
        };
        let other_index = self.get_mut(main_index)
            .as_ref().unwrap()
            .main_position(loglen)
            .unwrap_or(main_index);
        if other_index == main_index {
            let link = free_index as i32 - main_index as i32;
            *self.get_mut(free_index) = Some(
                self.get_mut(main_index)
                    .replace(item.into_item(link))
                    .unwrap()
                    .relocate(main_index, free_index)
            );
            return;
        }
        let mut prev_index = other_index;
        loop {
            let link = match self.get_mut(prev_index)
                .as_ref().unwrap()
            {
                &Item::Dead { link } | &Item::Live { link, .. }
                    if link != 0 => link,
                _ => panic!("table invariant is broken"),
            };
            let Some(next_index) = prev_index.checked_add_signed(link) else {
                panic!("integer overflow")
            };
            if next_index == main_index {
                break;
            }
            prev_index = next_index;
        }
        *self.get_mut(free_index) = Some(
            self.get_mut(main_index)
                .replace(item.into_item(0))
                .unwrap()
                .relocate(main_index, free_index)
        );
        self.get_mut(prev_index).as_mut().unwrap()
            .relocate_link(main_index, free_index);
    }

    fn get(&self, index: u32) -> &Option<Item<V>> {
        let items = self.table.items.as_ref().unwrap();
        &items[index as usize]
    }

    fn get_mut(&mut self, index: u32) -> &mut Option<Item<V>> {
        let items = self.table.items.as_mut().unwrap();
        &mut items[index as usize]
    }

    fn find_free_index(&mut self) -> Option<u32> {
        while self.table.last_free > 0 {
            self.table.last_free -= 1;
            if self.get(self.table.last_free).is_none() {
                return Some(self.table.last_free);
            }
        }
        None
    }

}

impl<V> IntoIterator for Table<V> {
    type Item = Option<Item<V>>;
    type IntoIter = TableIntoIter<V>;
    fn into_iter(self) -> Self::IntoIter {
        let items = self.items.map(Vec::from).map(Vec::into_iter);
        TableIntoIter { items }
    }
}

/// Table iterator of (key, value) pairs,
/// suitable for conversion into a map.
pub(super) struct TableIntoIter<V> {
    items: Option<std::vec::IntoIter<Option<Item<V>>>>,
}

impl<V> Iterator for TableIntoIter<V> {
    type Item = Option<Item<V>>;
    fn next(&mut self) -> Option<Self::Item> {
        self.items.as_mut().and_then(Iterator::next)
    }
}

impl<'s, V> IntoIterator for &'s Table<V> {
    type Item = &'s Option<Item<V>>;
    type IntoIter = TableIter<'s, V>;
    fn into_iter(self) -> Self::IntoIter {
        let items = self.items.as_ref().map(Box::as_ref).map(<[_]>::iter);
        TableIter { items }
    }
}

impl<V> Table<V> {
    pub fn iter(&'_ self) -> TableIter<'_, V> {
        <&Self as IntoIterator>::into_iter(self)
    }
}

/// Table iterator of (key, value) pairs,
/// suitable for representing as a map.
pub(super) struct TableIter<'s, V> {
    items: Option<std::slice::Iter<'s, Option<Item<V>>>>,
}

impl<'s, V> Iterator for TableIter<'s, V> {
    type Item = &'s Option<Item<V>>;
    fn next(&mut self) -> Option<Self::Item> {
        self.items.as_mut().and_then(Iterator::next)
    }
}

impl<V> Table<V> {
    pub fn sorted_iter(&'_ self) -> TableSortedIter<'_, V> {
        TableSortedIter { state: TableSortedIterState::Table(self) }
    }
}

/// Table iterator of (key, value) pairs,
/// suitable for serializing as a map.
/// Outputs keys in a predictable order.
pub(super) struct TableSortedIter<'s, V> {
    state: TableSortedIterState<'s, V>,
}

pub(super) enum TableSortedIterState<'s, V> {
    Table(&'s Table<V>),
    Items(std::vec::IntoIter<&'s Option<Item<V>>>),
}

impl<'s, V> Iterator for TableSortedIter<'s, V> {
    type Item = &'s Option<Item<V>>;
    fn next(&mut self) -> Option<Self::Item> {
        use TableSortedIterState as State;
        match self.state {
            State::Items(ref mut items) => items,
            State::Table(table) => {
                let mut items: Vec<_> = table.iter().collect();
                items.sort_unstable_by_key( |&item_opt|
                    item_opt.as_ref().and_then(|item| match item {
                        Item::Dead { .. } => None,
                        Item::Live { key, .. }
                            => Some(key),
                    })
                );
                self.state = State::Items(items.into_iter());
                #[allow(clippy::shadow_unrelated)]
                let State::Items(ref mut items) = &mut self.state else {
                    unreachable!()
                };
                items
            },
        }.next()
    }
}

pub(super) mod load {

use crate::{
    common::{u32_to_usize, LogSize},
    load::Error,
};

use super::{Item, Table};

/// Table builder for the purposes of Load trait
pub(in super::super) struct TableLoadBuilder<V> {
    table: Table<V>,
}

impl<V> TableLoadBuilder<V> {

    pub(crate) fn new(loglen: Option<LogSize>) -> Self {
        Self { table: Table::new(loglen) }
    }

    pub(crate) fn build<E: Error>(self)
    -> Result<Table<V>, E>
    {
        #[allow(clippy::assertions_on_constants)]
        { assert!(u32::BITS <= usize::BITS); }
        if self.table.len() > u32::MAX as usize {
            return Err(E::from(
                "the table should not be that large" ));
        }
        if self.table.last_free > self.table.len() as u32 {
            return Err(E::from(
                "last free index should not exceed table size" ));
        }
        self.table.validate_positions::<E>()?;
        Ok(self.table)
    }

    pub(crate) fn insert(&mut self, index: u32, item: Item<V>) {
        let items = self.table.items.as_mut().unwrap();
        let index = u32_to_usize(index);
        let old_item = items[index].replace(item);
        assert!(old_item.is_none());
    }

    pub(crate) fn set_last_free(&mut self, last_free: u32) {
        self.table.last_free = last_free;
    }

}

}

pub(super) mod dump {

use crate::common::LogSize;

use super::{Item, Table};

impl<V> Table<V> {
    pub(in super::super) fn dump_iter(&self) -> TableDumpIter<'_, V> {
        TableDumpIter {
            items: self.items.as_ref().map_or_else(
                Default::default,
                |items| items.iter() ),
            loglen: self.loglen(),
            last_free: self.last_free(),
        }
    }
}

pub(in super::super) struct TableDumpIter<'s, V> {
    items: std::slice::Iter<'s, Option<Item<V>>>,
    loglen: Option<LogSize>,
    last_free: u32,
}

impl<'s, V> Clone for TableDumpIter<'s, V> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            loglen: self.loglen,
            last_free: self.last_free,
        }
    }
}

impl<'s, V> TableDumpIter<'s, V> {
    pub(crate) fn loglen(&self) -> Option<LogSize> {
        self.loglen
    }
    pub(crate) fn last_free(&self) -> u32 {
        self.last_free
    }
    #[inline]
    fn convert_item(item: &Option<Item<V>>) -> Option<Item<&V>> {
        item.as_ref().map(Item::as_value_ref)
    }
}

impl<'s, V> Iterator for TableDumpIter<'s, V> {
    type Item = Option<Item<&'s V>>;
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.next()?;
        Some(Self::convert_item(item))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        Some(Self::convert_item(self.items.nth(n)?))
    }
}

impl<'s, V> ExactSizeIterator for TableDumpIter<'s, V> {
    fn len(&self) -> usize {
        self.items.len()
    }
}

}

#[cfg(test)]
mod test {
#![allow(clippy::string_slice)]
#![allow(clippy::cast_lossless)]

use std::slice;

use crate::{
    error::LoadError,
    common::{LogSize, ilog2_ceil},
    string::Str,
};

use super::Key;

use super::TableBuilder;

fn test_positions_for_keys_with_len<S>(
    keys: impl Iterator<Item=S>,
    loglen: Option<LogSize>,
)
where Str: From<S>,
{
    let mut table = TableBuilder::<()>::new(loglen);
    for key in keys {
        table.insert(Key::Name(Str::from(key)), None);
    }
    let table = table.build();
    let err = table.validate_positions::<LoadError>();
    if err.is_err() {
        eprintln!("{table:?}");
    }
    err.unwrap();
}

fn test_positions_for_keys<S>(
    keys: impl ExactSizeIterator<Item=S>,
)
where Str: From<S>,
{
    let loglen = ilog2_ceil(keys.len());
    test_positions_for_keys_with_len(keys, loglen)
}

fn test_positions_spacious_for_keys<S>(
    keys: impl ExactSizeIterator<Item=S>,
)
where Str: From<S>,
{
    let loglen = ilog2_ceil(2*keys.len());
    test_positions_for_keys_with_len(keys, loglen)
}

#[test]
fn test_positions() {
    fn iter_prefix(aaa: &str) -> impl ExactSizeIterator<Item=&'_ str> {
        (0..aaa.len()).map(|i| &aaa[..=i])
    }
    fn iter_letters(abc: &str) -> impl ExactSizeIterator<Item=&'_ str> {
        abc.as_bytes().iter().map(
            |c| std::str::from_utf8(slice::from_ref(c)).unwrap()
        )
    }
    for aaa in [
        "a", "aa", "ab", "ba", "aaaa",
        "aaaaaaaa", "aaaaaaaaaaaaaaaa",
    ] {
        test_positions_for_keys(iter_prefix(aaa));
        test_positions_spacious_for_keys(iter_prefix(aaa));
    }
    for abc in [
        "abcdefgh", "abcdefghij",
        "ABCDEFGH", "ABCDEFGHIJ",
    ] {
        test_positions_for_keys(iter_letters(abc));
        test_positions_spacious_for_keys(iter_letters(abc));
    }
}

}

