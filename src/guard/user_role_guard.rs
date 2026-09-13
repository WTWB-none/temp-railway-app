use actix_web::guard::Guard;

use crate::database::user::{User, UserRoles};

pub struct UserRole;

impl Guard for UserRole {
    fn check(&self, ctx: &actix_web::guard::GuardContext<'_>) -> bool {
        ctx.req_data()
            .get::<User>()
            .is_some_and(|user| user.get_role() == &UserRoles::SuperAdmin)
    }
}
