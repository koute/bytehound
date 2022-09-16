#![feature(const_btree_new)]

use std::ops::Range;
use std::collections::BTreeMap;

// This was copied from `ahash`.
#[inline(always)]
const fn folded_multiply( s: u64, by: u64 ) -> u64 {
    let result = (s as u128).wrapping_mul( by as u128 );
    ((result & 0xffff_ffff_ffff_ffff) as u64) ^ ((result >> 64) as u64)
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
struct Index( u64 );

impl Index {
    #[inline(always)]
    fn new( value: u64 ) -> Self {
        Index( value )
    }

    #[inline(always)]
    fn get( self ) -> u64 {
        self.0
    }
}

#[derive(Clone)]
struct Node< K, V > {
    index: Index,
    key: K,
    value: V,
    prev: Option< Index >,
    next: Option< Index >,
}

impl< K, V > Node< K, V > {
    fn hasher( &self ) -> u64 {
        self.index.get()
    }
}

macro_rules! eq {
    ($index:expr) => {{
        let index = $index;
        move |node| node.index == index
    }}
}

#[derive(Clone)]
struct LinkedHashMap< K, V > {
    nodes: hashbrown::raw::RawTable< Node< K, V > >,
    counter: u64,
    first_and_last: Option< (Index, Index) >
}

impl< K, V > LinkedHashMap< K, V > {
    pub const fn new() -> Self {
        Self {
            nodes: hashbrown::raw::RawTable::new(),
            counter: 0,
            first_and_last: None
        }
    }

    pub fn is_empty( &self ) -> bool {
        self.nodes.is_empty()
    }

    pub fn len( &self ) -> usize {
        self.nodes.len()
    }

    #[inline(always)]
    fn generate_index( &mut self ) -> Index {
        self.counter += 1;
        Index::new( (folded_multiply( self.counter, 6364136223846793005 ) >> 32) | (self.counter << 32) )
    }

    pub fn insert_back( &mut self, key: K, value: V ) -> Index {
        let index = self.generate_index();

        let prev;
        if let Some( (_, ref mut last_index) ) = self.first_and_last {
            prev = Some( *last_index );
            self.nodes.get_mut( last_index.get(), eq!( *last_index ) ).unwrap().next = Some( index );
            *last_index = index;
        } else {
            prev = None;
            self.first_and_last = Some( (index, index) );
        }

        let node = Node {
            index,
            key,
            value,
            prev,
            next: None
        };
        self.nodes.insert_entry( index.get(), node, Node::hasher );

        index
    }

    pub fn insert_front( &mut self, key: K, value: V ) -> Index {
        let index = self.generate_index();

        let next;
        if let Some( (ref mut first_index, _) ) = self.first_and_last {
            next = Some( *first_index );
            self.nodes.get_mut( first_index.get(), eq!( *first_index ) ).unwrap().prev = Some( index );
            *first_index = index;
        } else {
            next = None;
            self.first_and_last = Some( (index, index) );
        }

        let node = Node {
            index,
            key,
            value,
            prev: None,
            next
        };
        self.nodes.insert_entry( index.get(), node, Node::hasher );

        index
    }

    pub fn insert_before( &mut self, next_index: Index, key: K, value: V ) -> Index {
        let next_node = self.nodes.get( next_index.get(), eq!( next_index ) ).expect( "provided index doesn't exist" );
        if let Some( prev_index ) = next_node.prev {
            let index = self.generate_index();
            let node = Node {
                index,
                key,
                value,
                prev: Some( prev_index ),
                next: Some( next_index )
            };
            self.nodes.insert_entry( index.get(), node, Node::hasher );

            self.nodes.get_mut( prev_index.get(), eq!( prev_index ) ).unwrap().next = Some( index );
            self.nodes.get_mut( next_index.get(), eq!( next_index ) ).unwrap().prev = Some( index );

            return index;
        }

        assert_eq!( self.first_and_last.unwrap().0, next_index );
        self.insert_front( key, value )
    }

    pub fn insert_after( &mut self, prev_index: Index, key: K, value: V ) -> Index {
        let prev_node = self.nodes.get( prev_index.get(), eq!( prev_index ) ).expect( "provided index doesn't exist" );
        if let Some( next_index ) = prev_node.next {
            let index = self.generate_index();
            let node = Node {
                index,
                key,
                value,
                prev: Some( prev_index ),
                next: Some( next_index )
            };
            self.nodes.insert_entry( index.get(), node, Node::hasher );

            self.nodes.get_mut( prev_index.get(), eq!( prev_index ) ).unwrap().next = Some( index );
            self.nodes.get_mut( next_index.get(), eq!( next_index ) ).unwrap().prev = Some( index );

            return index;
        }

        assert_eq!( self.first_and_last.unwrap().1, prev_index );
        self.insert_back( key, value )
    }

    pub fn remove_and_get_next_index( &mut self, index: Index ) -> Option< Index > {
        let node = self.nodes.remove_entry( index.get(), eq!( index ) ).unwrap();
        match (node.prev, node.next) {
            (Some( prev_index ), Some( next_index )) => {
                self.nodes.get_mut( prev_index.get(), eq!( prev_index ) ).unwrap().next = Some( next_index );
                self.nodes.get_mut( next_index.get(), eq!( next_index ) ).unwrap().prev = Some( prev_index );
                Some( next_index )
            },
            (None, None) => {
                self.first_and_last = None;
                None
            },
            (Some( prev_index ), None) => {
                self.nodes.get_mut( prev_index.get(), eq!( prev_index ) ).unwrap().next = None;
                self.first_and_last.as_mut().unwrap().1 = prev_index;
                None
            },
            (None, Some( next_index )) => {
                self.nodes.get_mut( next_index.get(), eq!( next_index ) ).unwrap().prev = None;
                self.first_and_last.as_mut().unwrap().0 = next_index;
                Some( next_index )
            }
        }
    }

    pub fn next_index( &self, index: Index ) -> Option< Index > {
        self.nodes.get( index.get(), eq!( index ) ).unwrap().next
    }

    pub fn first_index( &self ) -> Option< Index > {
        self.first_and_last.map( |(first, _)| first )
    }

    pub fn first_and_last_index( &self ) -> Option< (Index, Index) > {
        self.first_and_last
    }

    pub fn get_key( &self, index: Index ) -> &K {
        &self.nodes.get( index.get(), eq!( index ) ).unwrap().key
    }

    pub fn get_key_mut( &mut self, index: Index ) -> &mut K {
        &mut self.nodes.get_mut( index.get(), eq!( index ) ).unwrap().key
    }

    pub fn get_value( &self, index: Index ) -> &V {
        &self.nodes.get( index.get(), eq!( index ) ).unwrap().value
    }

    pub fn get_value_mut( &mut self, index: Index ) -> &mut V {
        &mut self.nodes.get_mut( index.get(), eq!( index ) ).unwrap().value
    }

    pub fn into_vec( mut self ) -> Vec< (K, V) > {
        let mut output = Vec::with_capacity( self.len() );
        let mut index_opt = self.first_index();
        while let Some( index ) = index_opt {
            let node = self.nodes.remove_entry( index.get(), eq!( index ) ).unwrap();
            output.push( (node.key, node.value) );
            index_opt = node.next;
        }

        output
    }
}

#[derive(Clone)]
pub struct RangeMap< V > {
    map: BTreeMap< u64, Index >,
    data: LinkedHashMap< Range< u64 >, V >,
}

impl< V > Default for RangeMap< V > {
    fn default() -> Self {
        RangeMap {
            map: Default::default(),
            data: LinkedHashMap::new()
        }
    }
}

impl< V > RangeMap< V > {
    pub const fn new() -> Self {
        RangeMap {
            map: BTreeMap::new(),
            data: LinkedHashMap::new(),
        }
    }

    pub fn is_empty( &self ) -> bool {
        self.data.is_empty()
    }

    pub fn len( &self ) -> usize {
        self.data.len()
    }

    fn find_starting_index( &self, key: Range< u64 > ) -> Option< Index > {
        // This finds the first entry where `entry.end > key.start`.
        self.map.range( key.start + 1.. ).next().map( |(_, index)| *index )
    }

    fn insert_at_starting_index( &mut self, mut index_opt: Option< Index >, key: Range< u64 >, value: V ) where V: Clone {
        // The new key starts *before* this range ends.

        // All possibilities:
        //
        // ---------
        //     |OOO|    (new range is added before the old range; fin)
        // |NNN|
        // ---------
        // |OOOOOOO|    (old range is kept, but is chopped into two pieces; fin)
        //   |NNN|
        // ---------
        //   |OOOOO|    (old range is kept, but is chopped at the start; fin)
        // |NNN|
        // ---------
        // |OOOOO|      (old range is kept, but is chopped at the start; fin)
        // |NNN|
        // ---------
        // |OOOOO|      (old range is kept, but is chopped at the end; fin)
        //   |NNN|
        // ---------
        // |OOOOO|??    (old range is kept, but is chopped at the end; continue)
        //     |NNN|
        // ---------
        // |OOO|????    (old range is not kept; fin)
        // |NNN|
        // ---------
        // |OOO|????    (old range is not kept; continue)
        // |NNNNNNN|
        // ---------
        //   |OOO|??    (old range is not kept; continue)
        // |NNNNNNN|

        loop {
            let index = match index_opt {
                Some( index ) => index,
                None => {
                    let new_index = self.data.insert_back( key.clone(), value );
                    self.map.insert( key.end, new_index );
                    break;
                }
            };

            let old = self.data.get_key( index ).clone();
            if key.end <= old.start {
                // The new key ends *before* this range starts, so there's no overlap.
                // It should be inserted before this range.
                //
                //     |OOO|
                // |NNN|
                //
                let new_index = self.data.insert_before( index, key.clone(), value );
                self.map.insert( key.end, new_index );
                return;
            }


            if old.start >= key.start && old.end <= key.end {
                // The old range is completely covered by the new one.

                if old.end == key.end {
                    // |OOO|????    (old range is not kept; fin)
                    // |NNN|
                    *self.data.get_key_mut( index ) = key;
                    *self.data.get_value_mut( index ) = value;
                    return;
                }

                // |OOO|????    (old range is not kept; continue)
                // |NNNNNNN|
                // ---------
                //   |OOO|??    (old range is not kept; continue)
                // |NNNNNNN|

                index_opt = self.data.remove_and_get_next_index( index );
                self.map.remove( &old.end );
                continue;
            }

            // The old range is partially covered by the new one.

            if key.start > old.start && key.end < old.end {
                // |OOOOOOO|    (old range is kept, but is chopped into two pieces; fin)
                //   |NNN|
                //

                let old_value = self.data.get_value( index ).clone();
                self.data.get_key_mut( index ).end = key.start;
                self.map.remove( &old.end );
                self.map.insert( key.start, index );
                let new_index_1 = self.data.insert_after( index, key.clone(), value );
                let new_index_2 = self.data.insert_after( new_index_1, key.end..old.end, old_value );
                self.map.insert( key.end, new_index_1 );
                self.map.insert( old.end, new_index_2 );
                return;
            }

            if key.start <= old.start && key.end > old.start {
                //   |OOOOO|    (old range is kept, but is chopped at the start; fin)
                // |NNN|
                // ---------
                // |OOOOO|      (old range is kept, but is chopped at the start; fin)
                // |NNN|
                self.data.get_key_mut( index ).start = key.end;
                let new_index = self.data.insert_before( index, key.clone(), value );
                self.map.insert( key.end, new_index );
                return;
            }

            if key.end == old.end {
                // |OOOOO|    (old range is kept, but is chopped at the end; fin)
                //   |NNN|
                self.data.get_key_mut( index ).end = key.start;
                let new_index = self.data.insert_after( index, key.clone(), value );
                self.map.remove( &old.end );
                self.map.insert( key.start, index );
                self.map.insert( key.end, new_index );
                return;
            }

            // ---------
            // |OOOOO|??    (old range is kept, but is chopped at the end; continue)
            //     |NNN|

            self.data.get_key_mut( index ).end = key.start;
            self.map.remove( &old.end );
            self.map.insert( key.start, index );
            index_opt = self.data.next_index( index );
        }
    }

    pub fn insert( &mut self, key: Range< u64 >, value: V ) where V: Clone {
        if key.start == key.end {
            return;
        }
        assert!( key.start < key.end );

        if let Some( (first_index, last_index) ) = self.data.first_and_last_index() {
            if key.start >= self.data.get_key( last_index ).end {
                let index = self.data.insert_back( key.clone(), value );
                self.map.insert( key.end, index );
                return;
            } else if key.end <= self.data.get_key( first_index ).start {
                let index = self.data.insert_front( key.clone(), value );
                self.map.insert( key.end, index );
                return;
            }

            let index = self.find_starting_index( key.clone() );
            self.insert_at_starting_index( index, key, value );
        } else {
            let index = self.data.insert_back( key.clone(), value );
            self.map.insert( key.end, index );
        }
    }

    pub fn from_vec( vec: Vec< (Range< u64 >, V) > ) -> Self where V: Clone {
        let mut map = Self::new();
        map.data.nodes.reserve( vec.len(), Node::hasher );

        for (key, value) in vec {
            map.insert( key, value );
        }

        map
    }

    pub fn into_vec( self ) -> Vec< (Range< u64 >, V) > {
        self.data.into_vec()
    }

    pub fn get_value( &self, key: u64 ) -> Option< &V > {
        let mut iter = self.map.range( key.. );
        let mut index = *iter.next()?.1;
        let mut range = self.data.get_key( index ).clone();

        if key == range.end {
            index = *iter.next()?.1;
            range = self.data.get_key( index ).clone();
        }

        if key >= range.start && key < range.end {
            return Some( self.data.get_value( index ) );
        }

        None
    }

    pub fn values( &self ) -> impl ExactSizeIterator< Item = &V > {
        struct ValuesIter< 'a, V > {
            map: &'a RangeMap< V >,
            index: Option< Index >,
            remaining: usize
        }

        impl< 'a, V > Iterator for ValuesIter< 'a, V > {
            type Item = &'a V;

            fn next( &mut self ) -> Option< Self::Item > {
                let index = self.index?;
                self.index = self.map.data.next_index( index );
                Some( self.map.data.get_value( index ) )
            }

            fn size_hint( &self ) -> (usize, Option< usize >) {
                (self.remaining, Some( self.remaining ))
            }
        }

        impl< 'a, V > ExactSizeIterator for ValuesIter< 'a, V > {}

        ValuesIter {
            map: self,
            index: self.data.first_index(),
            remaining: self.data.len()
        }
    }
}

#[test]
fn test_insert_overlapping_at_the_start() {
    let mut map = RangeMap::new();
    map.insert( 2..10, 0 );
    map.insert( 0..4, 1 );
    assert_eq!( map.into_vec(), vec![
        ((0..4), 1),
        ((4..10), 0)
    ]);
}

#[test]
fn test_insert_overlapping_at_the_end() {
    let mut map = RangeMap::new();
    map.insert( 0..4, 0 );
    map.insert( 2..10, 1 );
    assert_eq!( map.into_vec(), vec![
        ((0..2), 0),
        ((2..10), 1)
    ]);
}

#[test]
fn test_insert_exactly_overlapping() {
    let mut map = RangeMap::new();
    map.insert( 2..10, 0 );
    map.insert( 2..10, 1 );
    assert_eq!( map.into_vec(), vec![
        ((2..10), 1)
    ]);
}

#[test]
fn test_insert_bigger_overlapping() {
    let mut map = RangeMap::new();
    map.insert( 4..6, 0 );
    map.insert( 2..8, 1 );
    assert_eq!( map.into_vec(), vec![
        ((2..8), 1)
    ]);
}

#[test]
fn test_insert_smaller_overlapping() {
    let mut map = RangeMap::new();
    map.insert( 2..8, 0 );
    map.insert( 4..6, 1 );
    assert_eq!( map.into_vec(), vec![
        ((2..4), 0),
        ((4..6), 1),
        ((6..8), 0),
    ]);
}

#[test]
fn test_insert_longer_then_shorter() {
    let mut map = RangeMap::new();
    map.insert( 2..8, 0 );
    map.insert( 2..6, 1 );
    assert_eq!( map.into_vec(), vec![
        ((2..6), 1),
        ((6..8), 0),
    ]);
}

#[test]
fn test_overlapping_two() {
    let mut map = RangeMap::new();
    map.insert( 4..30, 0 );
    map.insert( 8..20, 1 );
    assert_eq!( map.clone().into_vec(), vec![
        ((4..8), 0),
        ((8..20), 1),
        ((20..30), 0),
    ]);

    map.insert( 4..16, 2 );
    assert_eq!( map.into_vec(), vec![
        ((4..16), 2),
        ((16..20), 1),
        ((20..30), 0),
    ]);
}

#[test]
fn test_case_1() {
    let mut map = RangeMap::new();
    map.insert( 0..10, 0 );
    map.insert( 8..12, 1 );
    map.insert( 0..1, 2 );
    map.insert( 8..18, 3 );

    assert_eq!( map.into_vec(), vec![
        ((0..1), 2),
        ((1..8), 0),
        ((8..18), 3),
    ]);
}

#[test]
fn test_get_value() {
    let mut map = RangeMap::new();
    map.insert( 1..8, 0 );
    map.insert( 10..18, 1 );
    map.insert( 18..28, 2 );

    assert_eq!( map.get_value( 0 ).copied(), None );
    assert_eq!( map.get_value( 1 ).copied(), Some( 0 ) );
    assert_eq!( map.get_value( 2 ).copied(), Some( 0 ) );
    assert_eq!( map.get_value( 7 ).copied(), Some( 0 ) );
    assert_eq!( map.get_value( 8 ).copied(), None );
    assert_eq!( map.get_value( 9 ).copied(), None );
    assert_eq!( map.get_value( 10 ).copied(), Some( 1 ) );
    assert_eq!( map.get_value( 17 ).copied(), Some( 1 ) );
    assert_eq!( map.get_value( 18 ).copied(), Some( 2 ) );
    assert_eq!( map.get_value( 19 ).copied(), Some( 2 ) );
    assert_eq!( map.get_value( 27 ).copied(), Some( 2 ) );
    assert_eq!( map.get_value( 28 ).copied(), None );
}
