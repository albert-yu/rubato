use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, patch, post},
    Form, Router,
};
use askama::Template;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres, migrate::MigrateDatabase};
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(sqlx::FromRow, serde::Serialize)]
struct Todo {
    id: i32,
    task: String,
    completed: bool,
}

#[derive(serde::Deserialize)]
struct CreateTodo {
    task: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    todos: Vec<Todo>,
}

#[derive(Template)]
#[template(path = "todo_item.html")]
struct TodoItemTemplate {
    todo: Todo,
}

struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> axum::response::Response {
        match self.0.render() {
            Ok(html) => axum::response::Html(html).into_response(),
            Err(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "rubato=debug,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    if !Postgres::database_exists(&db_url).await.unwrap_or(false) {
        tracing::info!("Creating database {}", db_url);
        Postgres::create_database(&db_url).await?;
    } else {
        tracing::info!("Database already exists");
    }

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;
    tracing::info!("Migrations executed successfully.");

    let app = Router::new()
        .route("/", get(index))
        .route("/todos", post(add_todo))
        .route("/todos/{id}", patch(toggle_todo).delete(delete_todo))
        .nest_service("/assets", ServeDir::new("assets"))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(pool);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn index(State(pool): State<Pool<Postgres>>) -> impl IntoResponse {
    let todos = sqlx::query_as::<_, Todo>("SELECT id, task, completed FROM todos ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();

    HtmlTemplate(IndexTemplate { todos })
}

async fn add_todo(
    State(pool): State<Pool<Postgres>>,
    Form(form): Form<CreateTodo>,
) -> impl IntoResponse {
    let todo = sqlx::query_as::<_, Todo>(
        "INSERT INTO todos (task) VALUES ($1) RETURNING id, task, completed",
    )
    .bind(form.task)
    .fetch_one(&pool)
    .await
    .unwrap();

    HtmlTemplate(TodoItemTemplate { todo })
}

async fn toggle_todo(
    State(pool): State<Pool<Postgres>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let todo = sqlx::query_as::<_, Todo>(
        "UPDATE todos SET completed = NOT completed WHERE id = $1 RETURNING id, task, completed",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();

    HtmlTemplate(TodoItemTemplate { todo })
}

async fn delete_todo(State(pool): State<Pool<Postgres>>, Path(id): Path<i32>) -> impl IntoResponse {
    sqlx::query("DELETE FROM todos WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

    // Return empty OK to remove the element
    axum::http::StatusCode::OK
}