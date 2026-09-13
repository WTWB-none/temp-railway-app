DEFINE TABLE IF NOT EXISTS jwt_sessions SCHEMAFULL;

DEFINE FIELD user_email
ON TABLE jwt_sessions
TYPE string;

DEFINE FIELD access_token
ON TABLE jwt_sessions
TYPE string;

DEFINE FIELD refresh_token
ON TABLE jwt_sessions
TYPE string;

DEFINE INDEX IF NOT EXISTS jwt_session_user_email
ON TABLE jwt_sessions
FIELDS user_email;
