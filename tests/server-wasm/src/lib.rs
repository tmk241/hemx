use hemplate::Hemplate;
use hemx::Effect;

#[hemx::surface]
pub mod ui {}

#[derive(Hemplate)]
pub struct Room {
    pub count: i32,
}

#[derive(Hemplate)]
pub struct CounterView {
    pub count: i32,
}

pub fn render_page(count: i32) -> hemx::Html {
    ui::room::page(&Room { count })
}

pub fn update_counter(count: i32) -> Effect {
    ui::room::put(ui::room::advanced::slots::counter, &CounterView { count })
}
