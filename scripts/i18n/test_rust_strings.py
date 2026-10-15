"""Regression checks for the localization audit's Rust lexer."""
import unittest
from rust_strings import strings, production_source


class RustStrings(unittest.TestCase):
    def test_comments_characters_and_raw_strings(self):
        source = '''// "ignored"
/* nested /* "ignored" */ comment */
let c = '{'; let a = "Hello\\nworld"; let b = r##"literal \\n "quote""##;
'''
        self.assertEqual([text for _, _, text in strings(source)], ['Hello\nworld', 'literal \\n "quote"'])

    def test_production_code_after_a_test_module_is_preserved(self):
        source = '''fn before() { label("Before"); }
#[cfg(test)]
mod tests { // unmatched { in a comment
  fn test() { let x = r#"}"#; let c = '}'; label("Test only"); }
}
fn after() { label("After"); }
'''
        result = production_source(source)
        self.assertEqual(len(source), len(result))
        self.assertEqual(source.count('\n'), result.count('\n'))
        self.assertEqual([text for _, _, text in strings(result)], ['Before', 'After'])


if __name__ == '__main__':
    unittest.main()
