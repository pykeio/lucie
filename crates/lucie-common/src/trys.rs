/// Stable equivalent of [try blocks](https://doc.rust-lang.org/beta/unstable-book/language-features/try-blocks.html).
///
/// ```
/// use std::num::ParseIntError;
///
/// use lucie_common::trys;
///
/// let result: Result<i32, ParseIntError> =
/// 	trys!({ Ok("1".parse::<i32>()? + "2".parse::<i32>()? + "3".parse::<i32>()?) });
/// assert_eq!(result, Ok(6));
///
/// let result: Result<i32, ParseIntError> =
/// 	trys!({ Ok("1".parse::<i32>()? + "foo".parse::<i32>()? + "3".parse::<i32>()?) });
/// assert!(result.is_err());
/// ```
#[macro_export]
macro_rules! trys {
	($block:block) => {
		(|| $block)()
	};
	(async $block:block) => {
		(async || $block)()
	};
	(async move $block:block) => {
		(async move || $block)()
	};
}

#[cfg(test)]
mod tests {
	#[test]
	fn test_trys_option() {
		fn option_returning_function() -> Option<()> {
			None
		}

		let foo = trys!({
			option_returning_function()?;
			Some(())
		});

		assert_eq!(foo, None);
	}
}
