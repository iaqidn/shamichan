use serde::{Deserialize, Serialize};
use std::{hash::Hash, hint::unreachable_unchecked, ops::AddAssign};

// We opt to store strings as String even at the overhead of needing to convert
// back nad forth to Vec<char> for multibyte unicode support because it reduces
// memory usage almost 4 times. These will be stored in memory extensively on
// the server and client.

/// Node of the post body tree
//
// TODO: bump allocation for entire tree to reduce allocation/deallocation
// overhead. Depends on https://github.com/rust-lang/rust/issues/32838
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum Node {
	/// No content
	Empty,

	/// Start a new line
	NewLine,

	/// Contains a list of child nodes.
	///
	/// A list with a single Node must be handled just like that singe Node.
	Children(Vec<Node>),

	/// Contains unformatted text. Can include newlines.
	Text(String),

	/// Link to another post
	PostLink {
		/// Post the link points to
		id: u64,

		/// Target post's parent thread
		///
		/// If thread = 0, link has not had it's parenthood looked up yet on the
		/// server
		thread: u64,

		/// Parent page of target post
		page: u32,
	},

	/// Hash command result
	Command(Command),

	/// External URL
	URL(String),

	/// Configured reference to URL
	Reference { label: String, url: String },

	/// Link to embedadble resource
	Embed(Embed),

	/// Programming code tags
	Code(String),

	/// Spoiler tags
	Spoiler(Box<Node>),

	/// Bold formatting tags
	Bold(Box<Node>),

	/// Italic formatting tags
	Italic(Box<Node>),

	/// Quoted Node list. Results from line starting with `>`.
	Quoted(Box<Node>),

	/// Node dependant on some database access or processing and pending
	/// finalization.
	Pending(PendingNode),
}

impl Default for Node {
	#[inline]
	fn default() -> Self {
		Self::Empty
	}
}

impl Node {
	/// Construct a new text node
	#[inline]
	pub fn text(s: impl Into<String>) -> Node {
		Node::Text(s.into())
	}

	/// Construct a new quoted node
	#[inline]
	pub fn quote(inner: Node) -> Node {
		Node::Quoted(inner.into())
	}

	/// Construct a new spoiler node
	#[inline]
	pub fn spoiler(inner: Node) -> Node {
		Node::Spoiler(inner.into())
	}

	/// Diff the new post body against the old
	pub fn diff(&self, new: &Self) -> Option<Patch> {
		use Node::*;

		match (self, new) {
			(Empty, Empty) | (NewLine, NewLine) => None,
			(Children(old), Children(new)) => {
				let mut patch = vec![];
				let mut truncate = None;
				let mut append = vec![];

				let mut old_it = old.iter();
				let mut new_it = new.iter();
				let mut i = 0;
				loop {
					match (old_it.next(), new_it.next()) {
						(Some(o), Some(n)) => {
							if let Some(p) = o.diff(n) {
								patch.push((i, p));
							}
						}
						(None, Some(n)) => {
							append.push(n.clone());
							append.extend(new_it.map(Clone::clone));
							break;
						}
						(Some(_), None) => {
							truncate = Some(i);
							break;
						}
						(None, None) => break,
					};
					i += 1;
				}

				if patch.is_empty() && truncate.is_none() && append.is_empty() {
					None
				} else {
					Some(Patch::Children {
						patch,
						truncate,
						append,
					})
				}
			}
			(Children(old), new @ _) if old.len() == 1 => old[0].diff(new),
			(old @ _, Children(new)) if new.len() == 1 => old.diff(&new[0]),
			(Text(old), Text(new))
			| (URL(old), URL(new))
			| (Code(old), Code(new)) => {
				// Hot path - most strings won't change and this will compare by
				// length first anyway
				if old == new {
					None
				} else {
					Some(Patch::Text(TextPatch::new(
						&old.chars().collect::<Vec<char>>(),
						&new.chars().collect::<Vec<char>>(),
					)))
				}
			}
			(Spoiler(old), Spoiler(new))
			| (Bold(old), Bold(new))
			| (Italic(old), Italic(new))
			| (Quoted(old), Quoted(new)) => {
				Self::diff(old, new).map(|p| Patch::Wrapped(p.into()))
			}
			(old @ _, new @ _) => {
				if old != new {
					Some(Patch::Replace(new.clone()))
				} else {
					None
				}
			}
		}
	}

	/// Apply a patch tree to a post body tree
	pub fn patch(&mut self, patch: Patch) -> Result<(), String> {
		Ok(match (self, patch) {
			(dst @ _, Patch::Replace(p)) => {
				*dst = p;
			}
			(
				Node::Children(dst),
				Patch::Children {
					patch,
					truncate,
					append,
				},
			) => {
				for (i, p) in patch {
					let l = dst.len();
					dst.get_mut(i)
						.ok_or_else(|| {
							format!("patch out of bounds: {} >= {}", i, l)
						})?
						.patch(p)?;
				}
				if let Some(len) = truncate {
					dst.truncate(len);
				}
				dst.extend(append);
			}

			// Real ugly shit because you can't bind both dst and the contents
			// of Node::Children at the same time
			(dst @ Node::Children(_), p @ _)
				if match &dst {
					Node::Children(v) => v.len() == 1,
					_ => unsafe { unreachable_unchecked() },
				} =>
			{
				*dst = match dst {
					Node::Children(v) => std::mem::take(&mut v[0]),
					_ => unsafe { unreachable_unchecked() },
				};
				dst.patch(p)?;
			}

			(dst @ _, p @ Patch::Children { .. }) => {
				*dst = Node::Children(vec![std::mem::take(dst)]);
				dst.patch(p)?;
			}
			(Node::Text(dst), Patch::Text(p))
			| (Node::URL(dst), Patch::Text(p))
			| (Node::Code(dst), Patch::Text(p)) => {
				let mut new =
					String::with_capacity(p.estimate_new_size(dst.len()));
				p.apply(&mut new, dst.chars());
				*dst = new;
			}
			(Node::Spoiler(old), Patch::Wrapped(p))
			| (Node::Bold(old), Patch::Wrapped(p))
			| (Node::Italic(old), Patch::Wrapped(p))
			| (Node::Quoted(old), Patch::Wrapped(p)) => {
				old.patch(*p)?;
			}
			(dst @ _, p @ _) => {
				return Err(format!(
					"node type mismatch: attempting to patch {:#?}\nwith {:#?}",
					dst, p
				));
			}
		})
	}
}

impl AddAssign<Node> for Node {
	/// If pushing a Children to a Children, the destination list is extended.
	/// If pushing a Text to a Text, the destination Text is extended.
	/// Conversions from non-Children and Empty is automatically handled.
	fn add_assign(&mut self, rhs: Node) {
		use Node::*;

		match (self, rhs) {
			(_, Empty) => (),
			(dst @ Empty, n @ _) => *dst = n,
			// Merge adjacent strings
			(Text(s), Text(n)) => *s += &n,
			(Children(v), Children(n)) => {
				let mut it = n.into_iter();
				match (v.last_mut(), it.next()) {
					// Merge adjacent strings
					(Some(Text(dst)), Some(Text(s))) => *dst += &s,
					(_, Some(n @ _)) => v.push(n),
					_ => (),
				};
				v.extend(it);
			}
			(Children(v), Text(s)) => match v.last_mut() {
				// Merge adjacent strings
				Some(Text(dst)) => *dst += &s,
				_ => v.push(Text(s)),
			},
			(Children(v), n @ _) => v.push(n),
			(dst @ _, n @ _) => {
				*dst = Node::Children(vec![std::mem::take(dst), n])
			}
		};
	}
}

#[inline]
fn add_str<T>(dst: &mut Node, rhs: T)
where
	T: AsRef<str> + Into<String>,
{
	use Node::*;

	match dst {
		Text(dst) => *dst += rhs.as_ref(),
		Children(v) => match v.last_mut() {
			Some(Text(dst)) => *dst += rhs.as_ref(),
			_ => v.push(Text(rhs.into())),
		},
		_ => {
			*dst += Node::Text(rhs.into());
		}
	};
}

impl AddAssign<&str> for Node {
	/// Avoids allocations in comparison to += Node.
	fn add_assign(&mut self, rhs: &str) {
		add_str(self, rhs);
	}
}

impl AddAssign<String> for Node {
	/// Avoids allocations in comparison to += Node.
	fn add_assign(&mut self, rhs: String) {
		add_str(self, rhs);
	}
}

impl AddAssign<u8> for Node {
	/// Avoids allocations in comparison to += Node.
	/// Must only be used with valid non-null ASCII bytes.
	fn add_assign(&mut self, rhs: u8) {
		*self += rhs as char;
	}
}

impl AddAssign<char> for Node {
	/// Avoids allocations in comparison to += Node.
	/// Must only be used with valid non-null ASCII characters.
	fn add_assign(&mut self, rhs: char) {
		use Node::*;

		match self {
			Text(dst) => dst.push(rhs),
			Children(v) => match v.last_mut() {
				Some(Text(dst)) => dst.push(rhs),
				_ => v.push(Text(rhs.into())),
			},
			_ => {
				*self += Node::Text(rhs.into());
			}
		};
	}
}

macro_rules! impl_ref_add_assign {
	($($typ:ty)+) => {
		$(
			impl AddAssign<$typ> for &mut Node {
				fn add_assign(&mut self, rhs: $typ) {
					**self += rhs;
				}
			}
		)+
	};
}
impl_ref_add_assign! {
	Node
	&str
	String
	u8
	char
}

/// Node dependant on some database access or processing and pending
/// finalization.
/// Used by the server. These must never make it to the client.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PendingNode {
	Flip,
	EightBall,
	Pyu,
	PCount,

	/// Seconds to count down
	Countdown(u64),

	/// Hours to ban self for
	Autobahn(u16),

	Dice {
		/// Amount to offset the sum of all throws by
		offset: i16,

		/// Faces of the die
		faces: u16,

		/// Rolls to perform
		rolls: u8,
	},

	/// Pending post location fetch from the DB
	PostLink(u64),
}

/// Hash command result
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Command {
	/// Describes the parameters and results of one dice throw
	Dice {
		/// Amount to offset the sum of all throws by
		offset: i16,

		/// Faces of the die
		faces: u16,

		/// Results of dice throws. One per throw.
		results: Vec<u16>,
	},

	/// Coin flip
	Flip(bool),

	/// #8ball random answer dispenser
	EightBall(String),

	/// Synchronized countdown timer
	Countdown {
		start: u32,
		/// Unix timestamp
		secs: u32,
	},

	/// Self ban for N hours
	Autobahn(u16),

	/// Don't ask
	Pyu(u64),

	/// Don't ask
	PCount(u64),
}

/// Embedded content providers
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Copy)]
#[serde(rename_all = "snake_case")]
pub enum EmbedProvider {
	YouTube,
	SoundCloud,
	Vimeo,
	Coub,
	Twitter,
	Imgur,
	BitChute,
	Invidious,
}

/// Describes and identifies a specific embedadble resource
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct Embed {
	pub provider: EmbedProvider,
	pub data: String,
}

/// Patch to apply to an existing node
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Patch {
	/// Replace node with new one
	Replace(Node),

	/// Partially modify an existing textual Node
	Text(TextPatch),

	/// Patch the contents of a wrapped Node like Spoiler, Quoted, Bold and
	/// Italic
	Wrapped(Box<Patch>),

	/// Descend deeper to patch children the specified order
	Children {
		/// First patch nodes at the specific indices
		patch: Vec<(usize, Patch)>,

		/// Then truncate child list to match this size
		truncate: Option<usize>,

		/// Then append these nodes
		append: Vec<Node>,
	},
}

/// Patch to apply to the text body of a post
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostBodyPatch {
	pub id: u64,
	pub patch: Patch,
}

/// Partially modify an existing string
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct TextPatch {
	/// Position to start the mutation at
	pub position: u16,

	/// Number of characters to remove after position
	pub remove: u16,

	/// Text to insert at position after removal
	pub insert: Vec<char>,
}

impl TextPatch {
	/// Generate a patch from 2 versions of a string split into chars for
	/// multibyte unicode compatibility
	pub fn new(old: &[char], new: &[char]) -> Self {
		/// Find the first differing character in 2 character iterators
		#[inline]
		fn diff_i<'a, 'b>(
			a: impl Iterator<Item = &'a char>,
			b: impl Iterator<Item = &'b char>,
		) -> usize {
			a.zip(b).take_while(|(a, b)| a == b).count()
		}

		let start = diff_i(old.iter(), new.iter());
		let end = diff_i(old[start..].iter().rev(), new[start..].iter().rev());
		Self {
			position: start as u16,
			remove: (old.len() - end - start) as u16,
			insert: new[start..new.len() - end].iter().copied().collect(),
		}
	}

	/// Apply text patch to an existing string
	pub fn apply(
		&self,
		dst: &mut impl Extend<char>,
		mut src: impl Iterator<Item = char>,
	) {
		for _ in 0..self.position {
			dst.extend(src.next());
		}
		dst.extend(self.insert.iter().copied());
		dst.extend(src.skip(self.remove as usize));
	}

	/// Estimate size of destination after patch, assuming all characters are
	// single byte - true more often than not
	pub fn estimate_new_size(&self, dst_size: usize) -> usize {
		let mut s = dst_size as i16;
		s -= self.remove as i16;
		s += self.insert.len() as i16;

		// Protect against client-side attacks
		match s {
			0..=2000 => s as usize,
			_ => dst_size,
		}
	}
}

#[cfg(test)]
mod test {
	use super::TextPatch;

	// Test diffing and patching nodes
	#[test]
	fn node_diff() {
		// TODO
	}

	// Test diffing and patching text
	macro_rules! test_text_diff {
		($(
			$name:ident(
				$in:literal
				($pos:literal $remove:literal $insert:literal)
				$out:literal
			)
		)+) => {
			$(
				#[test]
				fn $name() {
					let std_patch = TextPatch{
						position: $pos,
						remove: $remove,
						insert: $insert.chars().collect(),
					};

					macro_rules! to_chars {
						($src:literal) => {{
							&$src.chars().collect::<Vec<char>>()
						}};
					}
					assert_eq!(
						TextPatch::new(to_chars!($in), to_chars!($out)),
						std_patch,
					);

					let mut res = String::new();
					std_patch.apply(&mut res, $in.chars());
					assert_eq!(res.as_str(), $out);
				}
			)+
		};
	}

	test_text_diff! {
		append(
			"a"
			(1 0 "a")
			"aa"
		)
		prepend(
			"bc"
			(0 0 "a")
			"abc"
		)
		append_to_empty_body(
			""
			(0 0 "abc")
			"abc"
		)
		backspace(
			"abc"
			(2 1 "")
			"ab"
		)
		remove_one_from_front(
			"abc"
			(0 1 "")
			"bc"
		)
		remove_one_multibyte_char(
			"αΒΓΔ"
			(2 1 "")
			"αΒΔ"
		)
		inject_into_the_middle(
			"abc"
			(2 0 "abc")
			"ababcc"
		)
		inject_multibyte_into_the_middle(
			"αΒΓ"
			(2 0 "Δ")
			"αΒΔΓ"
		)
		replace_in_the_middle(
			"abc"
			(1 1 "d")
			"adc"
		)
		replace_multibyte_in_the_middle(
			"αΒΓ"
			(1 1 "Δ")
			"αΔΓ"
		)
		replace_suffix(
			"abc"
			(1 2 "de")
			"ade"
		)
		replace_prefix(
			"abc"
			(0 2 "de")
			"dec"
		)
	}
}
