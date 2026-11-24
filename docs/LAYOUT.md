# Source and runtime layout

The template separates institutional purpose, governance, research, product source, public surfaces, evidence, shared resources and archives. The manifest assigns those roles to relative directories, so an organization can rename its folders without building an operator home path into the checker.

The invented note under memory/records is retained as source evidence. Its summary stays outside the records folder. The records.entries manifest rule lets the checker recognize protected records and keep automatic fixes from changing them.

Source checkouts hold current code and build metadata. Runtime databases, credentials, model files and dependency caches belong in explicitly configured external locations. A repository release exports selected source, notices and synthetic examples. It does not need to duplicate runtime data or regenerateable build outputs.

Changing the manifest changes a declared layout. Verify both the manifest and its landing places using preflight before relying on it. Folder or tool names in this example express the template, not a universal institutional taxonomy.
