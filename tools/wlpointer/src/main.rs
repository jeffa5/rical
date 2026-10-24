//! Hold a virtual pointer open on a wlroots compositor and drive it from
//! commands on stdin, one per line:
//!
//!   move X Y     move to absolute output coordinates
//!   press        press the left button
//!   release      release the left button
//!   click        press and release
//!
//! The pointer stays attached for as long as the program runs, so clients
//! see a seat with a pointer (headless sway has none of its own).

use std::io::BufRead;

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_output, wl_pointer::ButtonState, wl_registry, wl_seat},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
    output_size: Option<(i32, i32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match interface.as_str() {
                "wl_seat" => state.seat = Some(registry.bind(name, 1, qh, ())),
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                "wl_output" => {
                    registry.bind::<wl_output::WlOutput, _, _>(name, 1, qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, .. } = event {
            state.output_size = Some((width, height));
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_seat::WlSeat, ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1);

fn main() {
    let conn = Connection::connect_to_env().expect("no wayland display");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap();
    queue.roundtrip(&mut state).unwrap();

    let manager = state
        .manager
        .take()
        .expect("compositor lacks zwlr_virtual_pointer_manager_v1");
    let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    let (width, height) = state.output_size.unwrap_or((1280, 860));
    conn.flush().unwrap();

    let mut time = 0u32;
    let mut tick = || {
        time += 10;
        time
    };
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let words = line.split_whitespace().collect::<Vec<_>>();
        match words.as_slice() {
            ["move", x, y] => {
                let (x, y) = (x.parse::<u32>().unwrap(), y.parse::<u32>().unwrap());
                pointer.motion_absolute(tick(), x, y, width as u32, height as u32);
            }
            ["press"] => pointer.button(tick(), BTN_LEFT, ButtonState::Pressed),
            ["release"] => pointer.button(tick(), BTN_LEFT, ButtonState::Released),
            ["click"] => {
                pointer.button(tick(), BTN_LEFT, ButtonState::Pressed);
                pointer.frame();
                pointer.button(tick(), BTN_LEFT, ButtonState::Released);
            }
            [] => continue,
            _ => {
                eprintln!("wlpointer: unknown command {line:?}");
                continue;
            }
        }
        pointer.frame();
        conn.flush().unwrap();
        queue.dispatch_pending(&mut state).unwrap();
    }
}
