#[macro_export]
macro_rules! int_like {
    ($new_type_name:ident, $backing_type: ident) => {
        #[derive(Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
        pub struct $new_type_name($backing_type);

        impl $new_type_name {
            #[allow(dead_code)]
            #[inline]
            pub const fn get(self) -> $backing_type {
                self.0
            }
            #[allow(dead_code)]
            #[inline]
            pub const fn new(x: $backing_type) -> Self {
                $new_type_name(x)
            }
        }

        impl ::core::convert::From<$backing_type> for $new_type_name {
            #[inline]
            fn from(inner: $backing_type) -> Self {
                Self::new(inner)
            }
        }
        impl ::core::convert::From<$new_type_name> for $backing_type {
            #[inline]
            fn from(wrapped: $new_type_name) -> Self {
                wrapped.get()
            }
        }
    };

    ($new_type_name:ident, $new_atomic_type_name: ident, $backing_type:ident, $backing_atomic_type:ident) => {
        int_like!($new_type_name, $backing_type);

        /// A mutable holder for T that can safely be shared among threads.
        /// Runtime equivalent to using `AtomicUsize`, just type-safer.
        pub struct $new_atomic_type_name {
            container: $backing_atomic_type,
        }

        impl $new_atomic_type_name {
            #[allow(dead_code)]
            #[inline]
            pub const fn new(x: $new_type_name) -> Self {
                $new_atomic_type_name {
                    container: $backing_atomic_type::new(x.get()),
                }
            }
            #[allow(dead_code)]
            #[inline]
            pub fn load(&self, order: ::core::sync::atomic::Ordering) -> $new_type_name {
                $new_type_name::from(self.container.load(order))
            }
            #[allow(dead_code)]
            #[inline]
            pub fn store(&self, val: $new_type_name, order: ::core::sync::atomic::Ordering) {
                self.container.store(val.into(), order)
            }
            #[allow(dead_code)]
            #[inline]
            pub fn swap(
                &self,
                val: $new_type_name,
                order: ::core::sync::atomic::Ordering,
            ) -> $new_type_name {
                $new_type_name::from(self.container.swap(val.into(), order))
            }
            #[allow(dead_code)]
            #[inline]
            pub fn compare_exchange(
                &self,
                current: $new_type_name,
                new: $new_type_name,
                success: ::core::sync::atomic::Ordering,
                failure: ::core::sync::atomic::Ordering,
            ) -> ::core::result::Result<$new_type_name, $new_type_name> {
                match self
                    .container
                    .compare_exchange(current.into(), new.into(), success, failure)
                {
                    Ok(result) => Ok($new_type_name::from(result)),
                    Err(result) => Err($new_type_name::from(result)),
                }
            }
            #[allow(dead_code)]
            #[inline]
            pub fn compare_exchange_weak(
                &self,
                current: $new_type_name,
                new: $new_type_name,
                success: ::core::sync::atomic::Ordering,
                failure: ::core::sync::atomic::Ordering,
            ) -> ::core::result::Result<$new_type_name, $new_type_name> {
                match self.container.compare_exchange_weak(
                    current.into(),
                    new.into(),
                    success,
                    failure,
                ) {
                    Ok(result) => Ok($new_type_name::from(result)),
                    Err(result) => Err($new_type_name::from(result)),
                }
            }
        }
        impl ::core::default::Default for $new_atomic_type_name {
            #[inline]
            fn default() -> Self {
                Self::new($new_type_name::new(0))
            }
        }
    };
}