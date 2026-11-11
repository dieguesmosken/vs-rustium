mod highlighter;

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::Arc,
};
use glyphon::{
    cosmic_text::{Attrs, Family},
    Color, FontSystem, Metrics, Resolution,
    Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
};
use highlighter::SyntaxHighlighter;
use ropey::Rope;
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowBuilder},
};

const FONT_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 22.0;
const GUTTER_WIDTH: f32 = 50.0;
const DEFAULT_TEXT_COLOR: Color = Color::rgb(220, 220, 220);
const INDENT: &str = "    ";
const MAX_UNDO_STEPS: usize = 256;

#[derive(Default)]
struct EditHistory {
    undo: VecDeque<(Rope, usize)>,
    redo: VecDeque<(Rope, usize)>,
}

impl EditHistory {
    fn record(&mut self, buffer: &Rope, cursor: usize) {
        if self.undo.len() == MAX_UNDO_STEPS {
            self.undo.pop_front();
        }
        self.undo.push_back((buffer.clone(), cursor));
        self.redo.clear();
    }

    fn undo(&mut self, buffer: &mut Rope, cursor: &mut usize) -> bool {
        let Some((previous_buffer, previous_cursor)) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push_back((buffer.clone(), *cursor));
        *buffer = previous_buffer;
        *cursor = previous_cursor;
        true
    }

    fn redo(&mut self, buffer: &mut Rope, cursor: &mut usize) -> bool {
        let Some((next_buffer, next_cursor)) = self.redo.pop_back() else {
            return false;
        };
        if self.undo.len() == MAX_UNDO_STEPS {
            self.undo.pop_front();
        }
        self.undo.push_back((buffer.clone(), *cursor));
        *buffer = next_buffer;
        *cursor = next_cursor;
        true
    }
}

struct EditorState {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,

    buffer: Rope,
    cursor_char_idx: usize,
    highlighter: SyntaxHighlighter,
    history: EditHistory,
    file_path: Option<PathBuf>,
    control_pressed: bool,
    shift_pressed: bool,
    modified: bool,
}

impl EditorState {
    async fn new(window: Arc<Window>, initial_text: String, file_path: Option<PathBuf>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Adaptador gráfico compatível não encontrado.");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats[0];

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let mut text_atlas = TextAtlas::new(&device, &queue, surface_format);
        let text_renderer = TextRenderer::new(&mut text_atlas, &device, wgpu::MultisampleState::default(), None);

        let buffer = Rope::from_str(&initial_text);
        let cursor_char_idx = buffer.len_chars();
        let highlighter = SyntaxHighlighter::new();

        let state = Self {
            window,
            device,
            queue,
            surface,
            config,
            font_system,
            swash_cache,
            text_atlas,
            text_renderer,
            buffer,
            cursor_char_idx,
            highlighter,
            history: EditHistory::default(),
            file_path,
            control_pressed: false,
            shift_pressed: false,
            modified: false,
        };
        state.update_title();
        state
    }

    fn update_title(&self) {
        let name = self
            .file_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "Untitled".into());
        let marker = if self.modified { " •" } else { "" };
        self.window
            .set_title(&format!("{name}{marker} - VS Rust Codium"));
    }

    fn save(&mut self) {
        let Some(path) = self.file_path.as_ref() else {
            eprintln!("Salvamento indisponível: abra o editor com o caminho do arquivo.");
            return;
        };

        match fs::write(path, self.buffer.to_string()) {
            Ok(()) => {
                self.modified = false;
                self.update_title();
            }
            Err(error) => eprintln!("Erro ao salvar {}: {error}", path.display()),
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn handle_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        if self.control_pressed {
            if let Key::Character(character) = &event.logical_key {
                if character.eq_ignore_ascii_case("s") {
                    self.save();
                } else if character.eq_ignore_ascii_case("z") {
                    let changed = if self.shift_pressed {
                        self.history
                            .redo(&mut self.buffer, &mut self.cursor_char_idx)
                    } else {
                        self.history
                            .undo(&mut self.buffer, &mut self.cursor_char_idx)
                    };
                    self.mark_modified(changed);
                } else if character.eq_ignore_ascii_case("y") {
                    let changed = self
                        .history
                        .redo(&mut self.buffer, &mut self.cursor_char_idx);
                    self.mark_modified(changed);
                }
            }
            return;
        }

        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                if self.cursor_char_idx > 0 {
                    self.record_edit();
                    self.buffer.remove(self.cursor_char_idx - 1..self.cursor_char_idx);
                    self.cursor_char_idx -= 1;
                    changed = true;
                }
            }
            Key::Named(NamedKey::Delete) => {
                if self.cursor_char_idx < self.buffer.len_chars() {
                    self.record_edit();
                    self.buffer.remove(self.cursor_char_idx..self.cursor_char_idx + 1);
                    changed = true;
                }
            }
            Key::Named(NamedKey::Enter) => {
                let line = self
                    .buffer
                    .line(self.buffer.char_to_line(self.cursor_char_idx))
                    .to_string();
                let indent: String = line
                    .chars()
                    .take_while(|character| matches!(character, ' ' | '\t'))
                    .collect();
                let insertion = format!("\n{indent}");
                self.record_edit();
                self.buffer.insert(self.cursor_char_idx, &insertion);
                self.cursor_char_idx += insertion.chars().count();
                changed = true;
            }
            Key::Named(NamedKey::Home) => {
                self.cursor_char_idx = self
                    .buffer
                    .line_to_char(self.buffer.char_to_line(self.cursor_char_idx));
            }
            Key::Named(NamedKey::End) => {
                let line_idx = self.buffer.char_to_line(self.cursor_char_idx);
                let line_start = self.buffer.line_to_char(line_idx);
                let mut line_end = line_start + self.buffer.line(line_idx).len_chars();
                while line_end > line_start
                    && matches!(self.buffer.char(line_end - 1), '\n' | '\r')
                {
                    line_end -= 1;
                }
                self.cursor_char_idx = line_end;
            }
            Key::Named(NamedKey::Tab) => {
                if self.shift_pressed {
                    let line_start = self
                        .buffer
                        .line_to_char(self.buffer.char_to_line(self.cursor_char_idx));
                    let remove_count = self
                        .buffer
                        .line(self.buffer.char_to_line(self.cursor_char_idx))
                        .chars()
                        .take(INDENT.chars().count())
                        .take_while(|character| matches!(character, ' ' | '\t'))
                        .count();
                    if remove_count > 0 {
                        self.record_edit();
                        self.buffer.remove(line_start..line_start + remove_count);
                        self.cursor_char_idx = self
                            .cursor_char_idx
                            .saturating_sub(remove_count)
                            .max(line_start);
                        changed = true;
                    }
                } else {
                    self.record_edit();
                    self.buffer.insert(self.cursor_char_idx, INDENT);
                    self.cursor_char_idx += INDENT.chars().count();
                    changed = true;
                }
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if self.cursor_char_idx > 0 {
                    self.cursor_char_idx -= 1;
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if self.cursor_char_idx < self.buffer.len_chars() {
                    self.cursor_char_idx += 1;
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                let current_line = self.buffer.char_to_line(self.cursor_char_idx);
                if current_line > 0 {
                    let col = self.cursor_char_idx - self.buffer.line_to_char(current_line);
                    let target_line = current_line - 1;
                    let target_line_len = self.buffer.line(target_line).len_chars().saturating_sub(1);
                    let new_col = col.min(target_line_len);
                    self.cursor_char_idx = self.buffer.line_to_char(target_line) + new_col;
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                let current_line = self.buffer.char_to_line(self.cursor_char_idx);
                if current_line + 1 < self.buffer.len_lines() {
                    let col = self.cursor_char_idx - self.buffer.line_to_char(current_line);
                    let target_line = current_line + 1;
                    let target_line_len = self.buffer.line(target_line).len_chars().saturating_sub(1);
                    let new_col = col.min(target_line_len);
                    self.cursor_char_idx = self.buffer.line_to_char(target_line) + new_col;
                }
            }
            Key::Character(c) => {
                self.record_edit();
                self.buffer.insert(self.cursor_char_idx, c.as_str());
                self.cursor_char_idx += c.chars().count();
                changed = true;
            }
            _ => {}
        }

        self.mark_modified(changed);
    }

    fn record_edit(&mut self) {
        self.history.record(&self.buffer, self.cursor_char_idx);
    }

    fn mark_modified(&mut self, changed: bool) {
        if changed {
            self.modified = true;
            self.update_title();
            self.window.request_redraw();
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        let total_lines = self.buffer.len_lines();
        let gutter_text: String = (1..=total_lines)
            .map(|n| format!("{:>3}\n", n))
            .collect();

        // 1. Buffer do Gutter (Numeração)
        let mut gutter_buffer = glyphon::Buffer::new(
            &mut self.font_system,
            Metrics::new(FONT_SIZE, LINE_HEIGHT),
        );
        gutter_buffer.set_size(&mut self.font_system, GUTTER_WIDTH, self.config.height as f32);
        gutter_buffer.set_text(
            &mut self.font_system,
            &gutter_text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        gutter_buffer.shape_until_scroll(&mut self.font_system);

        // 2. Buffer de Código com Highlighting
        let mut code_buffer = glyphon::Buffer::new(
            &mut self.font_system,
            Metrics::new(FONT_SIZE, LINE_HEIGHT),
        );
        let content_width = (self.config.width as f32 - GUTTER_WIDTH - 20.0).max(1.0);
        code_buffer.set_size(&mut self.font_system, content_width, self.config.height as f32);

        let content = self.buffer.to_string();
        let spans = self.highlighter.highlight(&content);

        // Constrói os trechos de texto fatiados pelas faixas do Tree-sitter
        let mut rich_text: Vec<(&str, Attrs)> = Vec::new();
        let mut last_byte = 0;

        for span in spans {
            if span.start_byte > last_byte && span.start_byte <= content.len() {
                rich_text.push((
                    &content[last_byte..span.start_byte],
                    Attrs::new().family(Family::Monospace).color(DEFAULT_TEXT_COLOR),
                ));
            }

            if span.end_byte <= content.len() && span.start_byte < span.end_byte {
                rich_text.push((
                    &content[span.start_byte..span.end_byte],
                    Attrs::new().family(Family::Monospace).color(span.color),
                ));
                last_byte = span.end_byte;
            }
        }

        if last_byte < content.len() {
            rich_text.push((
                &content[last_byte..],
                Attrs::new().family(Family::Monospace).color(DEFAULT_TEXT_COLOR),
            ));
        }

        code_buffer.set_rich_text(
            &mut self.font_system,
            rich_text,
            Shaping::Advanced,
        );
        code_buffer.shape_until_scroll(&mut self.font_system);

        self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.text_atlas,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
            [
                TextArea {
                    buffer: &gutter_buffer,
                    left: 10.0,
                    top: 10.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: GUTTER_WIDTH as i32 + 10,
                        bottom: self.config.height as i32,
                    },
                    default_color: Color::rgb(133, 133, 133),
                },
                TextArea {
                    buffer: &code_buffer,
                    left: GUTTER_WIDTH + 15.0,
                    top: 10.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: (GUTTER_WIDTH + 15.0) as i32,
                        top: 0,
                        right: self.config.width as i32,
                        bottom: self.config.height as i32,
                    },
                    default_color: DEFAULT_TEXT_COLOR,
                },
            ],
            &mut self.swash_cache,
        ).unwrap();

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Text Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.11,
                            g: 0.11,
                            b: 0.11,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            self.text_renderer.render(&self.text_atlas, &mut render_pass).unwrap();
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.text_atlas.trim();

        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_path = std::env::args_os().nth(1).map(PathBuf::from);
    let initial_text = match &file_path {
        Some(path) => match fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        },
        None => String::from(
            "// VS Rust Codium - Tree-sitter Syntax Highlighting\n\
             use std::collections::HashMap;\n\
             \n\
             fn main() {\n\
             \x20   let mut mapa: HashMap<&str, i32> = HashMap::new();\n\
             \x20   mapa.insert(\"chave\", 42);\n\
             \n\
             \x20   let numero = 123.45;\n\
             \x20   println!(\"Valores: {} e {}\", \"teste\", numero);\n\
             }\n",
        ),
    };

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("VS Rust Codium")
            .with_inner_size(winit::dpi::LogicalSize::new(1024, 768))
            .build(&event_loop)
            .unwrap(),
    );

    let mut state = pollster::block_on(EditorState::new(
        window.clone(),
        initial_text,
        file_path,
    ));

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent { window_id, ref event } if window_id == state.window.id() => {
                match event {
                    WindowEvent::CloseRequested => target.exit(),
                    WindowEvent::Resized(physical_size) => {
                        state.resize(*physical_size);
                        state.window.request_redraw();
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        state.handle_key(event);
                        state.window.request_redraw();
                    }
                    WindowEvent::ModifiersChanged(modifiers) => {
                        state.control_pressed = modifiers.state().control_key();
                        state.shift_pressed = modifiers.state().shift_key();
                    }
                    WindowEvent::RedrawRequested => {
                        match state.render() {
                            Ok(_) => {}
                            Err(wgpu::SurfaceError::Lost) => state.resize(state.window.inner_size()),
                            Err(wgpu::SurfaceError::OutOfMemory) => target.exit(),
                            Err(e) => eprintln!("Erro ao renderizar: {:?}", e),
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EditHistory;
    use ropey::Rope;

    #[test]
    fn undo_and_redo_restore_buffer_and_cursor() {
        let mut history = EditHistory::default();
        let mut buffer = Rope::from_str("hello");
        let mut cursor = 5;

        history.record(&buffer, cursor);
        buffer.insert(cursor, "!");
        cursor += 1;

        assert!(history.undo(&mut buffer, &mut cursor));
        assert_eq!(buffer, Rope::from_str("hello"));
        assert_eq!(cursor, 5);

        assert!(history.redo(&mut buffer, &mut cursor));
        assert_eq!(buffer, Rope::from_str("hello!"));
        assert_eq!(cursor, 6);
    }

    #[test]
    fn recording_an_edit_clears_redo_history() {
        let mut history = EditHistory::default();
        let mut buffer = Rope::from_str("a");
        let mut cursor = 1;

        history.record(&buffer, cursor);
        buffer.insert(cursor, "b");
        cursor += 1;
        assert!(history.undo(&mut buffer, &mut cursor));

        history.record(&buffer, cursor);
        buffer.insert(cursor, "c");
        cursor += 1;

        assert!(!history.redo(&mut buffer, &mut cursor));
        assert_eq!(buffer, Rope::from_str("ac"));
        assert_eq!(cursor, 2);
    }
}