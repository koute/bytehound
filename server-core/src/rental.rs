use std::iter::FusedIterator;
use std::sync::Arc;
use std::rc::Rc;
use std::ops::Deref;

use stable_deref_trait::StableDeref;

pub struct Rental< T, I > {
    pub container: T,
    pub inner: I
}

macro_rules! new_rental {
    ($container:expr, |$target: ident| $($rest:tt)*) => {{
        #[inline(always)]
        unsafe fn unsafe_deref< T: ::std::ops::Deref >( container: &T ) -> &'static T::Target {
            &*(container.deref() as *const _)
        }

        let container = $container;

        // This is just here to make the borrow checker verify
        // that the derefed reference isn't saved somewhere
        // outside of the closure's scope.
        if false {
            let $target = ::std::ops::Deref::deref( &container );
            let inner = { $($rest)* };
            ::std::mem::drop( inner );
        }

        let $target = unsafe { unsafe_deref( &container ) };
        let inner = { $($rest)* };

        crate::rental::Rental {
            container,
            inner
        }
    }};
}

impl< T: StableDeref, I > Deref for Rental< T, I > {
    type Target = I;

    #[inline]
    fn deref( &self ) -> &Self::Target {
        &self.inner
    }
}

impl< T: StableDeref, I: Iterator > Iterator for Rental< T, I > {
    type Item = I::Item;

    #[inline]
    fn next( &mut self ) -> Option< Self::Item > {
        self.inner.next()
    }
}

impl< T: StableDeref, I: ExactSizeIterator > ExactSizeIterator for Rental< T, I > {
    #[inline]
    fn len( &self ) -> usize {
        self.inner.len()
    }
}

impl< T: StableDeref, I: DoubleEndedIterator > DoubleEndedIterator for Rental< T, I > {
    #[inline]
    fn next_back( &mut self ) -> Option< Self::Item > {
        self.inner.next_back()
    }
}

impl< T: StableDeref, I: FusedIterator > FusedIterator for Rental< T, I > {}

impl< T, I: Clone > Clone for Rental< Rc< T >, I > {
    #[inline]
    fn clone( &self ) -> Self {
        Rental {
            container: self.container.clone(),
            inner: self.inner.clone()
        }
    }
}

impl< T, I: Clone > Clone for Rental< Arc< T >, I > {
    #[inline]
    fn clone( &self ) -> Self {
        Rental {
            container: self.container.clone(),
            inner: self.inner.clone()
        }
    }
}
