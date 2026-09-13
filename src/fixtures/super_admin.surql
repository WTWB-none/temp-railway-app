-- Development-only account used to exercise SuperAdmin-protected endpoints.
-- Login: superadmin@gmail.com
-- Password: Admin123!
UPSERT users:test_super_admin CONTENT {
    email: "superadmin@gmail.com",
    password: "$argon2id$v=19$m=65536,t=4,p=1$d3BodnBJNUs3NDkzTExXRA$fevqq90e4q0uiq1XyOxLC/8xYe7a61J3CPe2KEhyHMM",
    registration_type: { Default: {} },
    psuid: NONE,
    role: { SuperAdmin: {} }
};
