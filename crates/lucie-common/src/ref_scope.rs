use std::{
	mem::ManuallyDrop,
	ops::{Deref, DerefMut},
	ptr
};

pub struct RefScope<'x, T, D>
where
	D: FnOnce(&mut T)
{
	value: &'x mut T,
	on_exit: ManuallyDrop<D>
}

impl<'x, T, D> RefScope<'x, T, D>
where
	D: FnOnce(&mut T)
{
	#[inline]
	pub const fn new(value: &'x mut T, on_exit: D) -> Self {
		Self {
			value,
			on_exit: ManuallyDrop::new(on_exit)
		}
	}
}

impl<'x, T, D> Deref for RefScope<'x, T, D>
where
	D: FnOnce(&mut T)
{
	type Target = T;

	#[inline(always)]
	fn deref(&self) -> &Self::Target {
		self.value
	}
}

impl<'x, T, D> DerefMut for RefScope<'x, T, D>
where
	D: FnOnce(&mut T)
{
	#[inline(always)]
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.value
	}
}

impl<'x, T, D> Drop for RefScope<'x, T, D>
where
	D: FnOnce(&mut T)
{
	#[inline]
	fn drop(&mut self) {
		(unsafe { ptr::read(&*self.on_exit) })(self.value)
	}
}
