DEFINE TABLE IF NOT EXISTS orders SCHEMAFULL;

DEFINE FIELD id
ON TABLE orders
TYPE uuid;

DEFINE FIELD products
ON TABLE orders
TYPE array<[record<products>, int]>;

DEFINE FIELD package_box
ON TABLE orders
TYPE array<[record<boxes>, int]>;

DEFINE FIELD status
ON TABLE orders
TYPE
    { Packed: {} }
  | { NotPacked: {} };
