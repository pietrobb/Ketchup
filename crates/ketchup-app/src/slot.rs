//! An `Option<E>` slot for mutually exclusive states: one enum `E` whose
//! variants each box one state type. Opening a state replaces whichever one
//! was open; reading, editing and closing name the state by its type.

/// A state type that lives in one variant of `E`.
pub(crate) trait Variant<E>: Sized + Into<E> {
    fn of(state: &E) -> Option<&Self>;
    fn of_mut(state: &mut E) -> Option<&mut Self>;
    fn from_state(state: E) -> Result<Self, E>;
}

/// Typed access to the slot; borrows only the slot, so the rest of the app
/// stays usable while a state is read or edited.
pub(crate) trait Slot<E> {
    fn get<D: Variant<E>>(&self) -> Option<&D>;
    fn get_mut<D: Variant<E>>(&mut self) -> Option<&mut D>;
    /// Open `state`, replacing whichever state was open.
    fn open<D: Variant<E>>(&mut self, state: D);
    /// Close the state if it is a `D`; another open state stays.
    fn close<D: Variant<E>>(&mut self);
    /// Close the state if it is a `D` and hand it back.
    fn remove<D: Variant<E>>(&mut self) -> Option<D>;
}

impl<E> Slot<E> for Option<E> {
    fn get<D: Variant<E>>(&self) -> Option<&D> {
        self.as_ref().and_then(D::of)
    }

    fn get_mut<D: Variant<E>>(&mut self) -> Option<&mut D> {
        self.as_mut().and_then(D::of_mut)
    }

    fn open<D: Variant<E>>(&mut self, state: D) {
        *self = Some(state.into());
    }

    fn close<D: Variant<E>>(&mut self) {
        if self.get::<D>().is_some() {
            *self = None;
        }
    }

    fn remove<D: Variant<E>>(&mut self) -> Option<D> {
        match D::from_state(self.take()?) {
            Ok(state) => Some(state),
            Err(other) => {
                *self = Some(other);
                None
            }
        }
    }
}

/// `slot_variants!(Enum { Type => Variant, Type => Variant(Nested::Inner) })`
/// implements [`Variant`] for each boxed state type of `Enum`.
macro_rules! slot_variants {
    ($enum:ident { $($ty:ty => $variant:ident $(($nested:ident :: $inner:ident))?),* $(,)? }) => {$(
        impl From<$ty> for $enum {
            fn from(state: $ty) -> Self {
                let state = Box::new(state);
                slot_variants!(@in $enum, $variant, $($nested::$inner,)? state)
            }
        }
        impl $crate::slot::Variant<$enum> for $ty {
            fn of(state: &$enum) -> Option<&Self> {
                match state {
                    slot_variants!(@in $enum, $variant, $($nested::$inner,)? state) => Some(state),
                    _ => None,
                }
            }
            fn of_mut(state: &mut $enum) -> Option<&mut Self> {
                match state {
                    slot_variants!(@in $enum, $variant, $($nested::$inner,)? state) => Some(state),
                    _ => None,
                }
            }
            fn from_state(state: $enum) -> Result<Self, $enum> {
                match state {
                    slot_variants!(@in $enum, $variant, $($nested::$inner,)? state) => Ok(*state),
                    other => Err(other),
                }
            }
        }
    )*};
    // `Enum::Variant(state)` or `Enum::Variant(Nested::Inner(state))`, as an
    // expression or a pattern.
    (@in $enum:ident, $variant:ident, $state:ident) => { $enum::$variant($state) };
    (@in $enum:ident, $variant:ident, $nested:ident :: $inner:ident, $state:ident) => {
        $enum::$variant($nested::$inner($state))
    };
}
pub(crate) use slot_variants;
