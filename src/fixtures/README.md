# Development fixtures

Fixtures in this directory are loaded automatically by debug builds after all
database migrations. Release builds do not load them. Every record uses a
stable ID and `UPSERT`, so fixtures can be applied repeatedly without creating
duplicates.

The development dataset contains:

- 4 products with different dimensions, markers, and stock quantities;
- 3 box sizes with stock quantities;
- 2 orders referencing those products and boxes;
- 1 SuperAdmin account.

Test SuperAdmin credentials:

- email: `superadmin@gmail.com`
- password: `Admin123!`
