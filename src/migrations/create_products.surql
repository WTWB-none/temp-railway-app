  DEFINE TABLE IF NOT EXISTS products SCHEMAFULL;

  DEFINE FIELD id
  ON TABLE products
  TYPE uuid;

  DEFINE FIELD name
  ON TABLE products
  TYPE string;

  DEFINE FIELD marker
  ON TABLE products
  TYPE
    option<array<
      { Weak: {} }
    | { Water: {} }
    | { Fire: {} }>>;

  DEFINE FIELD bounds
  ON TABLE products
  TYPE 
  	{
	  width: number,
	  height: number,
	  depth: number
	};

  DEFINE FIELD weight
  ON TABLE products
  TYPE number;

  DEFINE FIELD count
  ON TABLE products
  TYPE number;

  DEFINE FIELD volume
  ON TABLE products
  TYPE number;
