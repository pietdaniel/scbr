use std::fmt;
use std::path::Path;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tree_sitter::Language as TreeSitterLanguage;

/// Source languages supported by the analyzer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[value(name = "typescript", alias = "type-script", alias = "ts")]
    TypeScript,
    #[value(name = "javascript", alias = "java-script", alias = "js")]
    JavaScript,
    Python,
    #[value(alias = "golang")]
    Go,
    Rust,
    Java,
    Kotlin,
}

impl Language {
    pub const ALL: [Self; 7] = [
        Self::TypeScript,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::Rust,
        Self::Java,
        Self::Kotlin,
    ];

    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "py" | "pyw" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "rs" => Some(Self::Rust),
            "java" => Some(Self::Java),
            "kt" | "kts" => Some(Self::Kotlin),
            _ => None,
        }
    }

    pub fn tree_sitter_language(self, path: &Path) -> TreeSitterLanguage {
        match self {
            Self::TypeScript
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx")) =>
            {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            }
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        }
    }

    pub fn function_node_types(self) -> &'static [&'static str] {
        match self {
            Self::Python => &["function_definition", "lambda"],
            Self::Rust => &["function_item", "closure_expression"],
            Self::JavaScript | Self::TypeScript => &[
                "function_declaration",
                "function_expression",
                "generator_function_declaration",
                "generator_function",
                "arrow_function",
                "method_definition",
            ],
            Self::Go => &["function_declaration", "method_declaration", "func_literal"],
            Self::Java => &[
                "method_declaration",
                "constructor_declaration",
                "compact_constructor_declaration",
                "lambda_expression",
            ],
            Self::Kotlin => &[
                "function_declaration",
                "anonymous_function",
                "lambda_literal",
                "secondary_constructor",
                "getter",
                "setter",
            ],
        }
    }

    pub fn is_function(self, kind: &str) -> bool {
        self.function_node_types().contains(&kind)
    }

    pub fn is_decision(self, kind: &str) -> bool {
        match self {
            Self::Python => matches!(
                kind,
                "if_statement"
                    | "elif_clause"
                    | "for_statement"
                    | "while_statement"
                    | "except_clause"
                    | "conditional_expression"
                    | "case_clause"
                    | "assert_statement"
                    | "for_in_clause"
                    | "if_clause"
            ),
            Self::JavaScript | Self::TypeScript => matches!(
                kind,
                "if_statement"
                    | "for_statement"
                    | "for_in_statement"
                    | "while_statement"
                    | "do_statement"
                    | "catch_clause"
                    | "switch_case"
                    | "ternary_expression"
            ),
            Self::Go => matches!(
                kind,
                "if_statement"
                    | "for_statement"
                    | "expression_case"
                    | "type_case"
                    | "communication_case"
            ),
            Self::Rust => matches!(
                kind,
                "if_expression"
                    | "while_expression"
                    | "for_expression"
                    | "loop_expression"
                    | "match_arm"
                    | "try_expression"
            ),
            Self::Java => matches!(
                kind,
                "if_statement"
                    | "for_statement"
                    | "enhanced_for_statement"
                    | "while_statement"
                    | "do_statement"
                    | "catch_clause"
                    | "switch_label"
                    | "ternary_expression"
            ),
            Self::Kotlin => matches!(
                kind,
                "if_expression"
                    | "for_statement"
                    | "while_statement"
                    | "do_while_statement"
                    | "catch_block"
                    | "when_entry"
                    | "elvis_expression"
            ),
        }
    }

    pub fn is_boolean_expression(self, kind: &str) -> bool {
        match self {
            Self::Python => kind == "boolean_operator",
            Self::JavaScript | Self::TypeScript | Self::Go | Self::Rust | Self::Java => {
                kind == "binary_expression"
            }
            Self::Kotlin => matches!(
                kind,
                "binary_expression" | "conjunction_expression" | "disjunction_expression"
            ),
        }
    }

    pub fn is_nesting_decision(self, kind: &str) -> bool {
        self.is_decision(kind)
            && !matches!(
                kind,
                "assert_statement"
                    | "try_expression"
                    | "if_clause"
                    | "for_in_clause"
                    | "expression_case"
                    | "type_case"
                    | "communication_case"
                    | "switch_case"
                    | "switch_label"
                    | "match_arm"
                    | "when_entry"
            )
    }

    pub fn is_jump(self, kind: &str) -> bool {
        matches!(
            kind,
            "break_statement" | "continue_statement" | "break_expression" | "continue_expression"
        )
    }

    pub fn is_clone_candidate(self, kind: &str) -> bool {
        self.is_function(kind)
            || matches!(
                kind,
                "if_statement"
                    | "for_statement"
                    | "for_in_statement"
                    | "enhanced_for_statement"
                    | "while_statement"
                    | "do_statement"
                    | "if_expression"
                    | "for_expression"
                    | "while_expression"
                    | "loop_expression"
                    | "try_statement"
                    | "try_expression"
                    | "when_expression"
                    | "match_expression"
                    | "switch_expression"
            )
    }

    pub fn is_identifier(self, kind: &str) -> bool {
        matches!(
            kind,
            "identifier"
                | "field_identifier"
                | "property_identifier"
                | "type_identifier"
                | "simple_identifier"
                | "label_identifier"
        ) || kind.ends_with("_identifier")
            || kind.ends_with("_identifier_pattern")
    }

    pub fn is_literal(self, kind: &str) -> bool {
        matches!(
            kind,
            "true"
                | "false"
                | "none"
                | "null"
                | "nil"
                | "undefined"
                | "integer"
                | "float"
                | "number"
                | "decimal_integer_literal"
                | "hex_integer_literal"
                | "oct_integer_literal"
                | "binary_integer_literal"
                | "real_literal"
                | "character_literal"
                | "char_literal"
                | "string"
                | "string_literal"
                | "template_string"
                | "interpreted_string_literal"
                | "raw_string_literal"
        ) || kind.contains("string_literal")
            || kind.ends_with("_literal")
    }

    pub fn is_comment(self, kind: &str) -> bool {
        kind == "comment"
            || kind.ends_with("_comment")
            || matches!(kind, "shebang_line" | "hash_bang_line")
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Rust => "Rust",
            Self::Java => "Java",
            Self::Kotlin => "Kotlin",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::Language;
    use std::path::Path;

    #[test]
    fn detects_all_supported_extensions() {
        let cases = [
            ("a.ts", Language::TypeScript),
            ("a.tsx", Language::TypeScript),
            ("a.jsx", Language::JavaScript),
            ("a.pyw", Language::Python),
            ("a.go", Language::Go),
            ("a.rs", Language::Rust),
            ("A.java", Language::Java),
            ("build.kts", Language::Kotlin),
        ];
        for (path, expected) in cases {
            assert_eq!(Language::from_path(Path::new(path)), Some(expected));
        }
    }
}
