  DEFINE TABLE IF NOT EXISTS users SCHEMAFULL;

  DEFINE FIELD email
  ON TABLE users
  TYPE string;

  DEFINE FIELD password
  ON TABLE users
  TYPE option<string>;

  DEFINE FIELD registration_type
  ON TABLE users
  TYPE
      { YandexID: {} }
    | { Default: {} };


  DEFINE FIELD role
  ON TABLE users
  TYPE
      { SuperAdmin: {} }
    | { Employer: {} };

  DEFINE FIELD psuid
  ON TABLE users
  TYPE option<string>;

  DEFINE INDEX IF NOT EXISTS user_email_unique
  ON TABLE users
  FIELDS email
  UNIQUE;
