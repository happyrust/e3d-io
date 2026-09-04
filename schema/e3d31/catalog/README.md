# E3D 3.1 noun catalog

Live, read-only export from AMS on 2026-08-26:

- `noun_layout.json`: 1,933 public `DbElementType` nouns and 22,049 ordered
  attribute declarations.
- `noun_attr_fields.json`: 4,271 distinct attributes and all 203,454 exported
  `DbAttributeField` values.

These files are optional logical metadata. The parser continues to obtain
physical offset, packed-BOOL bit, compact/wide size, and explicit-vs-implicit
information from `desvir.dat` and `attlib.dat`. Internal templates such as
`DBMDAT` may therefore be descriptor-only and remain fully parseable.

Use them with:

```text
e3d-descriptor describe ... \
  --catalog-layout schema/e3d31/catalog/noun_layout.json \
  --catalog-fields schema/e3d31/catalog/noun_attr_fields.json
```

or let the pack supply both:

```text
e3d-descriptor describe --schema schema/e3d31 ...
```

The companion descriptor-slot live scan reached its full 300,000-element
budget but observed only one noun/table in that MDB traversal. It is retained
in the export evidence, not promoted here as a global physical catalog.
