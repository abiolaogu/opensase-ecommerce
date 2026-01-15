//! OpenSASE E-commerce - Self-hosted E-commerce Platform

use anyhow::Result;
use axum::{extract::{Path, Query, State}, http::StatusCode, response::IntoResponse, routing::{get, post, put, delete}, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid, pub sku: String, pub name: String, pub description: Option<String>,
    pub price: i64, pub compare_at_price: Option<i64>, pub currency: String,
    pub category_id: Option<Uuid>, pub inventory_quantity: i32, pub status: String,
    pub images: Vec<String>, pub tags: Vec<String>, pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Category { pub id: Uuid, pub name: String, pub slug: String, pub description: Option<String>, pub parent_id: Option<Uuid>, pub image_url: Option<String>, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Order {
    pub id: Uuid, pub order_number: String, pub customer_id: Option<Uuid>, pub customer_email: String,
    pub status: String, pub subtotal: i64, pub tax: i64, pub shipping: i64, pub total: i64, pub currency: String,
    pub shipping_address: serde_json::Value, pub billing_address: serde_json::Value,
    pub payment_status: String, pub fulfillment_status: String,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OrderItem { pub id: Uuid, pub order_id: Uuid, pub product_id: Uuid, pub sku: String, pub name: String, pub quantity: i32, pub unit_price: i64, pub total: i64 }

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CartItem { pub id: Uuid, pub session_id: String, pub product_id: Uuid, pub quantity: i32, pub created_at: DateTime<Utc> }

#[derive(Clone)] pub struct AppState { pub db: sqlx::PgPool, pub nats: Option<async_nats::Client> }

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).with(tracing_subscriber::fmt::layer()).init();
    let db = PgPoolOptions::new().max_connections(10).connect(&std::env::var("DATABASE_URL")?).await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    let nats = std::env::var("NATS_URL").ok().and_then(|url| futures::executor::block_on(async_nats::connect(&url)).ok());
    let state = AppState { db, nats };

    let app = Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({"status": "healthy", "service": "opensase-ecommerce"})) }))
        .route("/api/v1/products", get(list_products).post(create_product))
        .route("/api/v1/products/:id", get(get_product).put(update_product).delete(delete_product))
        .route("/api/v1/categories", get(list_categories).post(create_category))
        .route("/api/v1/categories/:id", get(get_category))
        .route("/api/v1/orders", get(list_orders).post(create_order))
        .route("/api/v1/orders/:id", get(get_order))
        .route("/api/v1/cart/:session", get(get_cart).post(add_to_cart).delete(clear_cart))
        .route("/api/v1/checkout", post(checkout))
        .layer(TraceLayer::new_for_http()).layer(CorsLayer::permissive()).with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8083".to_string());
    tracing::info!("🚀 OpenSASE E-commerce listening on 0.0.0.0:{}", port);
    axum::serve(tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?, app).await?;
    Ok(())
}

#[derive(Debug, Deserialize)] pub struct ListParams { pub page: Option<u32>, pub per_page: Option<u32>, pub category: Option<Uuid>, pub search: Option<String> }
#[derive(Debug, Serialize)] pub struct PaginatedResponse<T> { pub data: Vec<T>, pub total: i64, pub page: u32 }

async fn list_products(State(s): State<AppState>, Query(p): Query<ListParams>) -> Result<Json<PaginatedResponse<Product>>, (StatusCode, String)> {
    let page = p.page.unwrap_or(1).max(1); let per_page = p.per_page.unwrap_or(20).min(100);
    let products = sqlx::query_as::<_, Product>("SELECT * FROM products WHERE status = 'active' ORDER BY created_at DESC LIMIT $1 OFFSET $2")
        .bind(per_page as i64).bind(((page-1)*per_page) as i64).fetch_all(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM products WHERE status = 'active'").fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PaginatedResponse { data: products, total: total.0, page }))
}

async fn get_product(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Product>, (StatusCode, String)> {
    sqlx::query_as::<_, Product>("SELECT * FROM products WHERE id = $1").bind(id).fetch_optional(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.map(Json).ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))
}

#[derive(Debug, Deserialize)] pub struct CreateProductRequest { pub name: String, pub description: Option<String>, pub price: i64, pub category_id: Option<Uuid>, pub inventory_quantity: Option<i32> }

async fn create_product(State(s): State<AppState>, Json(r): Json<CreateProductRequest>) -> Result<(StatusCode, Json<Product>), (StatusCode, String)> {
    let sku = format!("SKU-{:08}", rand::random::<u32>());
    let p = sqlx::query_as::<_, Product>("INSERT INTO products (id, sku, name, description, price, currency, category_id, inventory_quantity, status, images, tags, metadata, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'NGN', $6, $7, 'active', '{}', '{}', '{}', NOW(), NOW()) RETURNING *")
        .bind(Uuid::now_v7()).bind(&sku).bind(&r.name).bind(&r.description).bind(r.price).bind(r.category_id).bind(r.inventory_quantity.unwrap_or(0))
        .fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(p)))
}

async fn update_product(State(s): State<AppState>, Path(id): Path<Uuid>, Json(r): Json<CreateProductRequest>) -> Result<Json<Product>, (StatusCode, String)> {
    let p = sqlx::query_as::<_, Product>("UPDATE products SET name = $2, description = $3, price = $4, category_id = $5, inventory_quantity = $6, updated_at = NOW() WHERE id = $1 RETURNING *")
        .bind(id).bind(&r.name).bind(&r.description).bind(r.price).bind(r.category_id).bind(r.inventory_quantity.unwrap_or(0))
        .fetch_optional(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    Ok(Json(p))
}

async fn delete_product(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("UPDATE products SET status = 'deleted' WHERE id = $1").bind(id).execute(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_categories(State(s): State<AppState>) -> Result<Json<Vec<Category>>, (StatusCode, String)> {
    let cats = sqlx::query_as::<_, Category>("SELECT * FROM categories ORDER BY name").fetch_all(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(cats))
}

async fn get_category(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Category>, (StatusCode, String)> {
    sqlx::query_as::<_, Category>("SELECT * FROM categories WHERE id = $1").bind(id).fetch_optional(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.map(Json).ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))
}

#[derive(Debug, Deserialize)] pub struct CreateCategoryRequest { pub name: String, pub description: Option<String>, pub parent_id: Option<Uuid> }

async fn create_category(State(s): State<AppState>, Json(r): Json<CreateCategoryRequest>) -> Result<(StatusCode, Json<Category>), (StatusCode, String)> {
    let slug = r.name.to_lowercase().replace(' ', "-");
    let c = sqlx::query_as::<_, Category>("INSERT INTO categories (id, name, slug, description, parent_id, created_at) VALUES ($1, $2, $3, $4, $5, NOW()) RETURNING *")
        .bind(Uuid::now_v7()).bind(&r.name).bind(&slug).bind(&r.description).bind(r.parent_id)
        .fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(c)))
}

async fn list_orders(State(s): State<AppState>, Query(p): Query<ListParams>) -> Result<Json<PaginatedResponse<Order>>, (StatusCode, String)> {
    let page = p.page.unwrap_or(1).max(1); let per_page = p.per_page.unwrap_or(20).min(100);
    let orders = sqlx::query_as::<_, Order>("SELECT * FROM orders ORDER BY created_at DESC LIMIT $1 OFFSET $2")
        .bind(per_page as i64).bind(((page-1)*per_page) as i64).fetch_all(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM orders").fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(PaginatedResponse { data: orders, total: total.0, page }))
}

async fn get_order(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Order>, (StatusCode, String)> {
    sqlx::query_as::<_, Order>("SELECT * FROM orders WHERE id = $1").bind(id).fetch_optional(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.map(Json).ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))
}

#[derive(Debug, Deserialize)] pub struct CreateOrderRequest { pub customer_email: String, pub items: Vec<OrderItemRequest>, pub shipping_address: serde_json::Value }
#[derive(Debug, Deserialize)] pub struct OrderItemRequest { pub product_id: Uuid, pub quantity: i32 }

async fn create_order(State(s): State<AppState>, Json(r): Json<CreateOrderRequest>) -> Result<(StatusCode, Json<Order>), (StatusCode, String)> {
    let order_num = format!("ORD-{:08}", rand::random::<u32>());
    let o = sqlx::query_as::<_, Order>("INSERT INTO orders (id, order_number, customer_email, status, subtotal, tax, shipping, total, currency, shipping_address, billing_address, payment_status, fulfillment_status, created_at, updated_at) VALUES ($1, $2, $3, 'pending', 0, 0, 0, 0, 'NGN', $4, '{}', 'pending', 'unfulfilled', NOW(), NOW()) RETURNING *")
        .bind(Uuid::now_v7()).bind(&order_num).bind(&r.customer_email).bind(&r.shipping_address)
        .fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(o)))
}

async fn get_cart(State(s): State<AppState>, Path(session): Path<String>) -> Result<Json<Vec<CartItem>>, (StatusCode, String)> {
    let items = sqlx::query_as::<_, CartItem>("SELECT * FROM cart_items WHERE session_id = $1").bind(&session).fetch_all(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items))
}

#[derive(Debug, Deserialize)] pub struct AddToCartRequest { pub product_id: Uuid, pub quantity: i32 }

async fn add_to_cart(State(s): State<AppState>, Path(session): Path<String>, Json(r): Json<AddToCartRequest>) -> Result<(StatusCode, Json<CartItem>), (StatusCode, String)> {
    let item = sqlx::query_as::<_, CartItem>("INSERT INTO cart_items (id, session_id, product_id, quantity, created_at) VALUES ($1, $2, $3, $4, NOW()) ON CONFLICT (session_id, product_id) DO UPDATE SET quantity = cart_items.quantity + $4 RETURNING *")
        .bind(Uuid::now_v7()).bind(&session).bind(r.product_id).bind(r.quantity)
        .fetch_one(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(item)))
}

async fn clear_cart(State(s): State<AppState>, Path(session): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    sqlx::query("DELETE FROM cart_items WHERE session_id = $1").bind(&session).execute(&s.db).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn checkout(State(_s): State<AppState>, Json(_r): Json<serde_json::Value>) -> impl IntoResponse {
    Json(serde_json::json!({"status": "checkout_initiated", "message": "Implement payment integration"}))
}
