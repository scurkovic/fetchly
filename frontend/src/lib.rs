use gloo_net::http::Request;
use leptos::either::Either;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::KeyboardEvent;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SearchProduct {
    name: String,
    brand: Option<String>,
    store_display: String,
    price: f64,
    unit_price: Option<f64>,
    unit: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SearchResponse {
    products: Vec<SearchProduct>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Item {
    id: String,
    query: String,
    priority: String,
    blacklisted_brands: Vec<String>,
    current_chain: Option<String>,
    current_product_name: Option<String>,
    current_brand: Option<String>,
    current_price: Option<f64>,
    current_unit_price: Option<f64>,
}

#[derive(Serialize)]
struct CreateItemBody<'a> {
    query: &'a str,
    priority: &'a str,
    blacklisted_brands: Vec<String>,
}

#[derive(Serialize)]
struct UpdateBrandsBody {
    brands: Vec<String>,
}

#[derive(Serialize)]
struct UpdatePriorityBody<'a> {
    priority: &'a str,
}

// ── API helpers ───────────────────────────────────────────────────────────────

async fn fetch_search(q: String) -> Vec<SearchProduct> {
    if q.len() < 2 {
        return vec![];
    }
    let encoded: String = q
        .chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect();
    let url = format!("/api/search?q={encoded}");
    match Request::get(&url).send().await {
        Ok(r) if r.ok() => r
            .json::<SearchResponse>()
            .await
            .map(|r| r.products)
            .unwrap_or_default(),
        _ => vec![],
    }
}

async fn fetch_items() -> Vec<Item> {
    match Request::get("/api/items").send().await {
        Ok(r) if r.ok() => r.json::<Vec<Item>>().await.unwrap_or_default(),
        _ => vec![],
    }
}

async fn api_create_item(query: String, priority: String) -> Result<Item, String> {
    let body = CreateItemBody {
        query: &query,
        priority: &priority,
        blacklisted_brands: vec![],
    };
    let resp = Request::post("/api/items")
        .json(&body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<Item>().await.map_err(|e| e.to_string())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn api_delete_item(id: String) -> Result<(), String> {
    let resp = Request::delete(&format!("/api/items/{id}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status() == 204 || resp.ok() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn api_refresh_item(id: String) -> Result<Item, String> {
    let resp = Request::post(&format!("/api/items/{id}/refresh"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<Item>().await.map_err(|e| e.to_string())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn api_update_brands(id: String, brands: Vec<String>) -> Result<Item, String> {
    let resp = Request::patch(&format!("/api/items/{id}/brands"))
        .json(&UpdateBrandsBody { brands })
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<Item>().await.map_err(|e| e.to_string())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn api_update_priority(id: String, priority: String) -> Result<Item, String> {
    let resp = Request::patch(&format!("/api/items/{id}/priority"))
        .json(&UpdatePriorityBody {
            priority: &priority,
        })
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.ok() {
        resp.json::<Item>().await.map_err(|e| e.to_string())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

// ── ItemCard ──────────────────────────────────────────────────────────────────

#[component]
fn ItemCard(
    item: RwSignal<Item>,
    on_delete: Callback<String>,
    on_update: Callback<Item>,
) -> impl IntoView {
    let (show_add_brand, set_show_add_brand) = signal(false);
    let (new_brand, set_new_brand) = signal(String::new());
    let (loading, set_loading) = signal(false);

    let handle_delete = move |_| {
        let id = item.get().id.clone();
        spawn_local(async move {
            let _ = api_delete_item(id.clone()).await;
            on_delete.run(id);
        });
    };

    let handle_refresh = move |_| {
        let id = item.get().id.clone();
        set_loading.set(true);
        spawn_local(async move {
            if let Ok(updated) = api_refresh_item(id).await {
                on_update.run(updated);
            }
            set_loading.set(false);
        });
    };

    let handle_toggle_priority = move |_| {
        let id = item.get().id.clone();
        let current = item.get().priority.clone();
        let next = if current == "immediate" {
            "soon".to_string()
        } else {
            "immediate".to_string()
        };
        spawn_local(async move {
            if let Ok(updated) = api_update_priority(id, next).await {
                on_update.run(updated);
            }
        });
    };

    let handle_remove_brand = move |brand: String| {
        let id = item.get().id.clone();
        let mut brands = item.get().blacklisted_brands.clone();
        brands.retain(|b| *b != brand);
        spawn_local(async move {
            if let Ok(updated) = api_update_brands(id, brands).await {
                on_update.run(updated);
            }
        });
    };

    let handle_add_brand = move || {
        let brand = new_brand.get();
        let brand = brand.trim().to_string();
        if brand.is_empty() {
            return;
        }
        let id = item.get().id.clone();
        let mut brands = item.get().blacklisted_brands.clone();
        if !brands.contains(&brand) {
            brands.push(brand);
        }
        set_new_brand.set(String::new());
        set_show_add_brand.set(false);
        spawn_local(async move {
            if let Ok(updated) = api_update_brands(id, brands).await {
                on_update.run(updated);
            }
        });
    };

    view! {
        <div class="item-card">
            <div class="item-header">
                <div>
                    <span class="item-query">{move || item.get().query}</span>
                    {" "}
                    <span
                        class=move || format!("badge badge-{}", item.get().priority)
                        style="cursor:pointer"
                        title="Click to toggle priority"
                        on:click=handle_toggle_priority
                    >
                        {move || item.get().priority}
                    </span>
                </div>
                <div class="item-actions">
                    <button
                        class="btn btn-ghost btn-sm"
                        title="Refresh"
                        disabled=move || loading.get()
                        on:click=handle_refresh
                    >
                        {move || if loading.get() { "⟳" } else { "↻" }}
                    </button>
                    <button class="btn btn-ghost btn-sm" title="Delete" on:click=handle_delete>
                        "✕"
                    </button>
                </div>
            </div>

            {move || {
                let it = item.get();
                match it.current_price {
                    Some(price) => {
                        let chain = it.current_chain.unwrap_or_default();
                        let product = it.current_product_name.unwrap_or_default();
                        let brand = it.current_brand.unwrap_or_default();
                        let brand_str = if brand.is_empty() { String::new() } else { format!(" ({})", brand) };
                        let unit_str = match it.current_unit_price {
                            Some(up) => format!(" · {:.2}/unit", up),
                            None => String::new(),
                        };
                        Either::Left(view! {
                            <div class="item-best">
                                <span class="item-best-price">{format!("{:.2} EUR", price)}</span>
                                " · "
                                <span class="item-best-chain">{chain}</span>
                                {unit_str}
                                <div class="item-best-product">{product}{brand_str}</div>
                            </div>
                        })
                    }
                    None => Either::Right(view! {
                        <p class="no-best">"Tražim cijenu..."</p>
                    }),
                }
            }}

            <div class="item-brands">
                {move || {
                    item.get()
                        .blacklisted_brands
                        .into_iter()
                        .map(|b| {
                            let b_clone = b.clone();
                            view! {
                                <span class="brand-tag">
                                    {b.clone()}
                                    <button
                                        class="brand-remove"
                                        on:click=move |_| handle_remove_brand(b_clone.clone())
                                    >
                                        "×"
                                    </button>
                                </span>
                            }
                        })
                        .collect_view()
                }}
                <button
                    class="btn btn-ghost btn-sm"
                    on:click=move |_| set_show_add_brand.set(!show_add_brand.get())
                >
                    {move || {
                        if item.get().blacklisted_brands.is_empty() {
                            "🚫 Blacklist brand"
                        } else {
                            "+ brand"
                        }
                    }}
                </button>
            </div>

            {move || {
                if show_add_brand.get() {
                    Either::Left(view! {
                        <div class="add-brand-form">
                            <input
                                type="text"
                                placeholder="Brand name..."
                                prop:value=move || new_brand.get()
                                on:input=move |e| set_new_brand.set(event_target_value(&e))
                                on:keydown=move |e: KeyboardEvent| {
                                    if e.key() == "Enter" {
                                        handle_add_brand();
                                    }
                                }
                            />
                            <button
                                class="btn btn-primary btn-sm"
                                on:click=move |_| handle_add_brand()
                            >
                                "Add"
                            </button>
                        </div>
                    })
                } else {
                    Either::Right(view! { <span></span> })
                }
            }}
        </div>
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

#[component]
fn App() -> impl IntoView {
    let (search_query, set_search_query) = signal(String::new());
    let (priority, set_priority) = signal("immediate".to_string());
    let (items, set_items) = signal(Vec::<Item>::new());
    let (adding, set_adding) = signal(false);
    let (add_error, set_add_error) = signal(String::new());

    // Load items on mount
    Effect::new(move |_| {
        spawn_local(async move {
            let loaded = fetch_items().await;
            set_items.set(loaded);
        });
    });

    // LocalResource doesn't require Serializable, ideal for CSR
    let search_results = LocalResource::new(move || fetch_search(search_query.get()));

    let do_add = move || {
        let q = search_query.get();
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        let p = priority.get();
        set_adding.set(true);
        set_add_error.set(String::new());
        spawn_local(async move {
            match api_create_item(q, p).await {
                Ok(item) => {
                    set_items.update(|items| items.insert(0, item));
                    set_search_query.set(String::new());
                }
                Err(e) => set_add_error.set(e),
            }
            set_adding.set(false);
        });
    };

    let handle_delete: Callback<String> = Callback::new(move |id: String| {
        set_items.update(|items| items.retain(|i| i.id != id));
    });

    let handle_update: Callback<Item> = Callback::new(move |updated: Item| {
        set_items.update(|items| {
            if let Some(pos) = items.iter().position(|i| i.id == updated.id) {
                items[pos] = updated;
            }
        });
    });

    view! {
        <div class="container">
            <h1>"🛒 Fetchly"</h1>

            <div class="add-form">
                <div class="add-form-row">
                    <input
                        type="text"
                        placeholder="Što tražiš? (npr. masline zelene, mlijeko...)"
                        prop:value=move || search_query.get()
                        on:input=move |e| set_search_query.set(event_target_value(&e))
                        on:keydown={
                            let do_add = do_add.clone();
                            move |e: KeyboardEvent| {
                                if e.key() == "Enter" {
                                    do_add();
                                }
                            }
                        }
                    />
                    <button
                        class="btn btn-primary"
                        disabled=move || adding.get()
                        on:click={
                            let do_add = do_add.clone();
                            move |_| do_add()
                        }
                    >
                        {move || if adding.get() { "Dodajem..." } else { "Dodaj" }}
                    </button>
                </div>

                <div class="priority-row">
                    <span style="font-size:.85rem;color:#666;align-self:center">"Prioritet:"</span>
                    <button
                        class=move || format!(
                            "priority-btn immediate{}",
                            if priority.get() == "immediate" { " active" } else { "" }
                        )
                        on:click=move |_| set_priority.set("immediate".to_string())
                    >
                        "🔴 Odmah"
                    </button>
                    <button
                        class=move || format!(
                            "priority-btn soon{}",
                            if priority.get() == "soon" { " active" } else { "" }
                        )
                        on:click=move |_| set_priority.set("soon".to_string())
                    >
                        "🟡 Uskoro"
                    </button>
                </div>

                {move || {
                    let err = add_error.get();
                    if err.is_empty() {
                        Either::Left(view! { <span></span> })
                    } else {
                        Either::Right(view! { <p class="error-msg">{err}</p> })
                    }
                }}
            </div>

            // Search preview
            {move || {
                let q = search_query.get();
                if q.len() < 2 {
                    Either::Left(view! { <span></span> })
                } else {
                    Either::Right(view! {
                        <Suspense fallback=move || view! {
                            <div class="search-results">
                                <p style="color:#999;font-size:.875rem;padding:8px 0">"Tražim..."</p>
                            </div>
                        }>
                            {move || Suspend::new(async move {
                                let products = search_results.await;
                                if products.is_empty() {
                                    Either::Left(view! {
                                        <div class="search-results">
                                            <p style="color:#999;font-size:.875rem;padding:8px 0">
                                                "Nema rezultata."
                                            </p>
                                        </div>
                                    })
                                } else {
                                    Either::Right(view! {
                                        <div class="search-results">
                                            {products
                                                .into_iter()
                                                .take(6)
                                                .map(|p| {
                                                    let brand = p.brand.unwrap_or_default();
                                                    let sub = if brand.is_empty() {
                                                        p.store_display.clone()
                                                    } else {
                                                        format!("{} · {}", brand, p.store_display)
                                                    };
                                                    let price_str = match (p.unit_price, p.unit.as_deref()) {
                                                        (Some(up), Some(u)) if u == "l" || u == "kg" =>
                                                            format!("{:.2} € ({:.2}/{})", p.price, up, u),
                                                        _ => format!("{:.2} €", p.price),
                                                    };
                                                    view! {
                                                        <div class="search-product">
                                                            <div class="search-product-info">
                                                                <div class="search-product-name">{p.name}</div>
                                                                <div class="search-product-sub">{sub}</div>
                                                            </div>
                                                            <span class="price-tag">{price_str}</span>
                                                        </div>
                                                    }
                                                })
                                                .collect_view()}
                                        </div>
                                    })
                                }
                            })}
                        </Suspense>
                    })
                }
            }}

            <h2>"Moj popis"</h2>
            <div class="items-list">
                {move || {
                    let current = items.get();
                    if current.is_empty() {
                        Either::Left(view! {
                            <div class="empty-state">
                                "Popis je prazan. Dodaj što trebaš kupiti!"
                            </div>
                        })
                    } else {
                        Either::Right(
                            current
                                .into_iter()
                                .map(|it| {
                                    let sig = RwSignal::new(it);
                                    view! {
                                        <ItemCard
                                            item=sig
                                            on_delete=handle_delete
                                            on_update=Callback::new(move |updated: Item| {
                                                sig.set(updated.clone());
                                                handle_update.run(updated);
                                            })
                                        />
                                    }
                                })
                                .collect_view(),
                        )
                    }
                }}
            </div>
        </div>
    }
}

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(|| view! { <App/> });
}
