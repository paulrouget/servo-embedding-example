extern crate glutin;
extern crate servo;

use servo::gl;
use glutin::GlContext;
use servo::BrowserId;
use servo::compositing::compositor_thread::EventLoopWaker;
use servo::compositing::windowing::{WindowEvent, WindowMethods};
use servo::euclid::{Point2D, Size2D, TypedPoint2D, TypedRect, TypedScale, TypedSize2D,
                    TypedVector2D};
use servo::ipc_channel::ipc;
use servo::msg::constellation_msg::{Key, KeyModifiers};
use servo::net_traits::net_error_list::NetError;
use servo::script_traits::{LoadData, TouchEventType};
use servo::servo_config::opts;
use servo::servo_config::resource_files::set_resources_path;
use servo::servo_geometry::DeviceIndependentPixel;
use servo::servo_url::ServoUrl;
use servo::style_traits::DevicePixel;
use servo::style_traits::cursor::CursorKind;
use std::env;
use std::rc::Rc;
use std::sync::Arc;

pub struct GlutinEventLoopWaker {
    proxy: Arc<glutin::EventsLoopProxy>,
}

impl EventLoopWaker for GlutinEventLoopWaker {
    // Use by servo to share the "event loop waker" across threads
    fn clone(&self) -> Box<EventLoopWaker + Send> {
        Box::new(GlutinEventLoopWaker {
            proxy: self.proxy.clone(),
        })
    }
    // Called by servo when the main thread needs to wake up
    fn wake(&self) {
        self.proxy.wakeup().expect("wakeup eventloop failed");
    }
}

struct Window {
    glutin_window: glutin::GlWindow,
    waker: Box<EventLoopWaker>,
    gl: Rc<gl::Gl>,
}

fn main() {
    println!("Servo version: {}", servo::config::servo_version());

    let mut event_loop = glutin::EventsLoop::new();

    let builder = glutin::WindowBuilder::new().with_dimensions(800, 600);
    let gl_version = glutin::GlRequest::Specific(glutin::Api::OpenGl, (3, 2));
    let context = glutin::ContextBuilder::new()
        .with_gl(gl_version)
        .with_vsync(true);
    let window = glutin::GlWindow::new(builder, context, &event_loop).unwrap();

    window.show();

    let gl = unsafe {
        window
            .context()
            .make_current()
            .expect("Couldn't make window current");
        gl::GlFns::load_with(|s| window.context().get_proc_address(s) as *const _)
    };

    let event_loop_waker = Box::new(GlutinEventLoopWaker {
        proxy: Arc::new(event_loop.create_proxy()),
    });

    let path = env::current_dir().unwrap().join("resources");
    let path = path.to_str().unwrap().to_string();
    set_resources_path(Some(path));
    opts::set_defaults(opts::default_opts());

    let window = Rc::new(Window {
        glutin_window: window,
        waker: event_loop_waker,
        gl: gl,
    });

    let mut servo = servo::Servo::new(window.clone());

    let url = ServoUrl::parse("https://servo.org").unwrap();
    let (sender, receiver) = ipc::channel().unwrap();
    servo.handle_events(vec![WindowEvent::NewBrowser(url, sender)]);
    let browser_id = receiver.recv().unwrap();
    servo.handle_events(vec![WindowEvent::SelectBrowser(browser_id)]);

    let mut pointer = (0.0, 0.0);

    event_loop.run_forever(|event| {
        // Blocked until user event or until servo unblocks it
        match event {
            // This is the event triggered by GlutinEventLoopWaker
            glutin::Event::Awakened => {
                servo.handle_events(vec![]);
            }

            // Mousemove
            glutin::Event::WindowEvent {
                event:
                    glutin::WindowEvent::CursorMoved {
                        position: (x, y), ..
                    },
                ..
            } => {
                pointer = (x, y);
                let event =
                    WindowEvent::MouseWindowMoveEventClass(TypedPoint2D::new(x as f32, y as f32));
                servo.handle_events(vec![event]);
            }

            // reload when R is pressed
            glutin::Event::WindowEvent {
                event:
                    glutin::WindowEvent::KeyboardInput {
                        input:
                            glutin::KeyboardInput {
                                state: glutin::ElementState::Pressed,
                                virtual_keycode: Some(glutin::VirtualKeyCode::R),
                                ..
                            },
                        ..
                    },
                ..
            } => {
                let event = WindowEvent::Reload(browser_id);
                servo.handle_events(vec![event]);
            }

            // Scrolling
            glutin::Event::WindowEvent {
                event: glutin::WindowEvent::MouseWheel { delta, phase, .. },
                ..
            } => {
                let pointer = TypedPoint2D::new(pointer.0 as i32, pointer.1 as i32);
                let (dx, dy) = match delta {
                    glutin::MouseScrollDelta::LineDelta(dx, dy) => {
                        (dx, dy * 38.0 /*line height*/)
                    }
                    glutin::MouseScrollDelta::PixelDelta(dx, dy) => (dx, dy),
                };
                let scroll_location =
                    servo::webrender_api::ScrollLocation::Delta(TypedVector2D::new(dx, dy));
                let phase = match phase {
                    glutin::TouchPhase::Started => TouchEventType::Down,
                    glutin::TouchPhase::Moved => TouchEventType::Move,
                    glutin::TouchPhase::Ended => TouchEventType::Up,
                    glutin::TouchPhase::Cancelled => TouchEventType::Up,
                };
                let event = WindowEvent::Scroll(scroll_location, pointer, phase);
                servo.handle_events(vec![event]);
            }
            glutin::Event::WindowEvent {
                event: glutin::WindowEvent::Resized(width, height),
                ..
            } => {
                let event = WindowEvent::Resize;
                servo.handle_events(vec![event]);
                window.glutin_window.resize(width, height);
            }
            _ => {}
        }
        glutin::ControlFlow::Continue
    });
}

impl WindowMethods for Window {
    fn prepare_for_composite(&self, _width: usize, _height: usize) -> bool {
        true
    }

    fn present(&self) {
        self.glutin_window.swap_buffers().unwrap();
    }

    fn supports_clipboard(&self) -> bool {
        false
    }

    fn create_event_loop_waker(&self) -> Box<EventLoopWaker> {
        self.waker.clone()
    }

    fn gl(&self) -> Rc<gl::Gl> {
        self.gl.clone()
    }

    fn hidpi_factor(&self) -> TypedScale<f32, DeviceIndependentPixel, DevicePixel> {
        TypedScale::new(self.glutin_window.hidpi_factor())
    }

    fn framebuffer_size(&self) -> TypedSize2D<u32, DevicePixel> {
        let (width, height) = self.glutin_window.get_inner_size().unwrap();
        let scale_factor = self.glutin_window.hidpi_factor() as u32;
        TypedSize2D::new(scale_factor * width, scale_factor * height)
    }

    fn window_rect(&self) -> TypedRect<u32, DevicePixel> {
        TypedRect::new(TypedPoint2D::new(0, 0), self.framebuffer_size())
    }

    fn size(&self) -> TypedSize2D<f32, DeviceIndependentPixel> {
        let (width, height) = self.glutin_window.get_inner_size().unwrap();
        TypedSize2D::new(width as f32, height as f32)
    }

    fn client_window(&self, _id: BrowserId) -> (Size2D<u32>, Point2D<i32>) {
        let (width, height) = self.glutin_window.get_inner_size().unwrap();
        let (x, y) = self.glutin_window.get_position().unwrap();
        (Size2D::new(width, height), Point2D::new(x as i32, y as i32))
    }

    fn set_inner_size(&self, _id: BrowserId, _size: Size2D<u32>) {}

    fn set_position(&self, _id: BrowserId, _point: Point2D<i32>) {}

    fn set_fullscreen_state(&self, _id: BrowserId, _state: bool) {}

    fn set_page_title(&self, _id: BrowserId, title: Option<String>) {
        self.glutin_window.set_title(match title {
            Some(ref title) => title,
            None => "",
        });
    }

    fn status(&self, _id: BrowserId, _status: Option<String>) {}

    fn allow_navigation(&self, _id: BrowserId, _url: ServoUrl, chan: ipc::IpcSender<bool>) {
        chan.send(true).ok();
    }

    fn load_start(&self, _id: BrowserId) {}

    fn load_end(&self, _id: BrowserId) {}

    fn load_error(&self, _id: BrowserId, _: NetError, _url: String) {}

    fn head_parsed(&self, _id: BrowserId) {}

    fn history_changed(&self, _id: BrowserId, _entries: Vec<LoadData>, _current: usize) {}

    fn set_cursor(&self, cursor: CursorKind) {
        let cursor = match cursor {
            CursorKind::Pointer => glutin::MouseCursor::Hand,
            _ => glutin::MouseCursor::Default,
        };
        self.glutin_window.set_cursor(cursor);
    }

    fn set_favicon(&self, _id: BrowserId, _url: ServoUrl) {}

    fn handle_key(
        &self,
        _id: Option<BrowserId>,
        _ch: Option<char>,
        _key: Key,
        _mods: KeyModifiers,
    ) {
    }

    fn handle_panic(&self, _id: BrowserId, _reason: String, _backtrace: Option<String>) {}

    fn screen_avail_size(&self, _id: BrowserId) -> Size2D<u32> {
        let monitor = self.glutin_window.get_current_monitor();
        let (monitor_width, monitor_height) = monitor.get_dimensions();
        Size2D::new(monitor_width, monitor_height)
    }

    fn screen_size(&self, _id: BrowserId) -> Size2D<u32> {
        let monitor = self.glutin_window.get_current_monitor();
        let (monitor_width, monitor_height) = monitor.get_dimensions();
        Size2D::new(monitor_width, monitor_height)
    }
}
