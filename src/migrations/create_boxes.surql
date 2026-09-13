  DEFINE TABLE IF NOT EXISTS boxes SCHEMAFULL;

  DEFINE FIELD id
  ON TABLE boxes
  TYPE uuid;

  DEFINE FIELD bounds
  ON TABLE boxes
  TYPE 
  	{
	  width: number,
	  height: number,
	  depth: number
	};

  DEFINE FIELD max_weight
  ON TABLE boxes
  TYPE number;

  DEFINE FIELD count
  ON TABLE boxes
  TYPE number;

  DEFINE FIELD volume
  ON TABLE boxes
  TYPE number;
