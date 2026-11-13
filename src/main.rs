mod highlighter;

use glyphon::{
    cosmic_text::{Attrs, Family, Wrap},
    Color, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer,
};
use highlighter::SyntaxHighlighter;
use ropey::Rope;
use std::{collections::VecDeque, fs, ops::Range, path::PathBuf, sync::Arc};
use winit::{
    dpi::PhysicalPosition,
    event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowBuilder},
};

const FONT_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 22.0;
const GUTTER_WIDTH: f32 = 50.0;
const DEFAULT_TEXT_COLOR: Color = Color::rgb(220, 220, 220);
const SELECTION_TEXT_COLOR: Color = Color::rgb(109, 224, 209);
const INDENT: &str = "    ";
const MAX_UNDO_STEPS: usize = 256;
const CHARACTER_WIDTH: f32 = FONT_SIZE * 0.6;

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
    selection_anchor: Option<usize>,
    cursor_position: PhysicalPosition<f64>,
    dragging_selection: bool,
    scroll_line: usize,
    highlighter: SyntaxHighlighter,
    history: EditHistory,
    file_path: Option<PathBuf>,
    control_pressed: bool,
    shift_pressed: bool,
    modified: bool,
    saved_buffer: Rope,
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
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        let buffer = Rope::from_str(&initial_text);
        let cursor_char_idx = buffer.len_chars();
        let saved_buffer = buffer.clone();
        let highlighter = SyntaxHighlighter::new();

        let mut state = Self {
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
            selection_anchor: None,
            cursor_position: PhysicalPosition::new(0.0, 0.0),
            dragging_selection: false,
            scroll_line: 0,
            highlighter,
            history: EditHistory::default(),
            file_path,
            control_pressed: false,
            shift_pressed: false,
            modified: false,
            saved_buffer,
        };
        state.ensure_cursor_visible();
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
            .set_title(&format!("{name}{marker} - VS Rustium"));
    }

    fn save(&mut self) {
        let Some(path) = self.file_path.as_ref() else {
            eprintln!("Salvamento indisponível: abra o editor com o caminho do arquivo.");
            return;
        };

        match fs::write(path, self.buffer.to_string()) {
            Ok(()) => {
                self.saved_buffer = self.buffer.clone();
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
            self.ensure_cursor_visible();
        }
    }

    fn handle_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        if self.control_pressed {
            match &event.logical_key {
                Key::Character(character) if character.eq_ignore_ascii_case("s") => self.save(),
                Key::Character(character) if character.eq_ignore_ascii_case("a") => {
                    self.selection_anchor = Some(0);
                    self.cursor_char_idx = self.buffer.len_chars();
                }
                Key::Character(character) if character.eq_ignore_ascii_case("z") => {
                    let changed = if self.shift_pressed {
                        self.history
                            .redo(&mut self.buffer, &mut self.cursor_char_idx)
                    } else {
                        self.history
                            .undo(&mut self.buffer, &mut self.cursor_char_idx)
                    };
                    self.mark_modified(changed);
                    self.selection_anchor = None;
                }
                Key::Character(character) if character.eq_ignore_ascii_case("y") => {
                    let changed = self
                        .history
                        .redo(&mut self.buffer, &mut self.cursor_char_idx);
                    self.mark_modified(changed);
                    self.selection_anchor = None;
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    let target = move_word_cursor(&self.buffer, self.cursor_char_idx, -1);
                    self.move_cursor(target);
                }
                Key::Named(NamedKey::ArrowRight) => {
                    let target = move_word_cursor(&self.buffer, self.cursor_char_idx, 1);
                    self.move_cursor(target);
                }
                _ => {}
            }
            self.ensure_cursor_visible();
            return;
        }

        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                if let Some(range) = self.selection_range() {
                    self.record_edit();
                    self.buffer.remove(range.clone());
                    self.cursor_char_idx = range.start;
                    self.selection_anchor = None;
                    changed = true;
                } else if self.cursor_char_idx > 0 {
                    self.record_edit();
                    self.buffer
                        .remove(self.cursor_char_idx - 1..self.cursor_char_idx);
                    self.cursor_char_idx -= 1;
                    changed = true;
                }
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(range) = self.selection_range() {
                    self.record_edit();
                    self.buffer.remove(range.clone());
                    self.cursor_char_idx = range.start;
                    self.selection_anchor = None;
                    changed = true;
                } else if self.cursor_char_idx < self.buffer.len_chars() {
                    self.record_edit();
                    self.buffer
                        .remove(self.cursor_char_idx..self.cursor_char_idx + 1);
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
                if let Some(range) = self.selection_range() {
                    self.buffer.remove(range.clone());
                    self.cursor_char_idx = range.start;
                }
                self.buffer.insert(self.cursor_char_idx, &insertion);
                self.cursor_char_idx += insertion.chars().count();
                self.selection_anchor = None;
                changed = true;
            }
            Key::Named(NamedKey::Home) => {
                let target = self
                    .buffer
                    .line_to_char(self.buffer.char_to_line(self.cursor_char_idx));
                self.move_cursor(target);
            }
            Key::Named(NamedKey::End) => {
                let line_idx = self.buffer.char_to_line(self.cursor_char_idx);
                let line_start = self.buffer.line_to_char(line_idx);
                let mut line_end = line_start + self.buffer.line(line_idx).len_chars();
                while line_end > line_start && matches!(self.buffer.char(line_end - 1), '\n' | '\r')
                {
                    line_end -= 1;
                }
                self.move_cursor(line_end);
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
                        self.selection_anchor = None;
                        changed = true;
                    }
                } else {
                    self.record_edit();
                    if let Some(range) = self.selection_range() {
                        self.buffer.remove(range.clone());
                        self.cursor_char_idx = range.start;
                    }
                    self.buffer.insert(self.cursor_char_idx, INDENT);
                    self.cursor_char_idx += INDENT.chars().count();
                    self.selection_anchor = None;
                    changed = true;
                }
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if self.cursor_char_idx > 0 {
                    self.move_cursor(self.cursor_char_idx - 1);
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if self.cursor_char_idx < self.buffer.len_chars() {
                    self.move_cursor(self.cursor_char_idx + 1);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                let current_line = self.buffer.char_to_line(self.cursor_char_idx);
                if current_line > 0 {
                    let col = self.cursor_char_idx - self.buffer.line_to_char(current_line);
                    let target_line = current_line - 1;
                    let target_line_len = self.line_content_len(target_line);
                    let new_col = col.min(target_line_len);
                    self.move_cursor(self.buffer.line_to_char(target_line) + new_col);
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                let current_line = self.buffer.char_to_line(self.cursor_char_idx);
                if current_line + 1 < self.buffer.len_lines() {
                    let col = self.cursor_char_idx - self.buffer.line_to_char(current_line);
                    let target_line = current_line + 1;
                    let target_line_len = self.line_content_len(target_line);
                    let new_col = col.min(target_line_len);
                    self.move_cursor(self.buffer.line_to_char(target_line) + new_col);
                }
            }
            Key::Named(NamedKey::PageUp) => {
                let line = self.buffer.char_to_line(self.cursor_char_idx);
                let target_line = line.saturating_sub(self.visible_line_count());
                let column = self.cursor_char_idx - self.buffer.line_to_char(line);
                let target_length = self.line_content_len(target_line);
                self.move_cursor(self.buffer.line_to_char(target_line) + column.min(target_length));
            }
            Key::Named(NamedKey::PageDown) => {
                let line = self.buffer.char_to_line(self.cursor_char_idx);
                let target_line = (line + self.visible_line_count())
                    .min(self.buffer.len_lines().saturating_sub(1));
                let column = self.cursor_char_idx - self.buffer.line_to_char(line);
                let target_length = self.line_content_len(target_line);
                self.move_cursor(self.buffer.line_to_char(target_line) + column.min(target_length));
            }
            Key::Character(c) => {
                self.record_edit();
                if let Some(range) = self.selection_range() {
                    self.buffer.remove(range.clone());
                    self.cursor_char_idx = range.start;
                }
                self.buffer.insert(self.cursor_char_idx, c.as_str());
                self.cursor_char_idx += c.chars().count();
                self.selection_anchor = None;
                changed = true;
            }
            _ => {}
        }

        self.mark_modified(changed);
        self.ensure_cursor_visible();
    }

    fn record_edit(&mut self) {
        self.history.record(&self.buffer, self.cursor_char_idx);
    }

    fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor_char_idx)
            .then_some(anchor.min(self.cursor_char_idx)..anchor.max(self.cursor_char_idx))
    }

    fn move_cursor(&mut self, target: usize) {
        if self.shift_pressed {
            self.selection_anchor.get_or_insert(self.cursor_char_idx);
        } else {
            self.selection_anchor = None;
        }
        self.cursor_char_idx = target.min(self.buffer.len_chars());
    }

    fn visible_line_count(&self) -> usize {
        ((self.config.height as f32 - 32.0) / LINE_HEIGHT)
            .floor()
            .max(1.0) as usize
    }

    fn ensure_cursor_visible(&mut self) {
        let line = self.buffer.char_to_line(self.cursor_char_idx);
        let visible_lines = self.visible_line_count();
        if line < self.scroll_line {
            self.scroll_line = line;
        } else if line >= self.scroll_line + visible_lines {
            self.scroll_line = line + 1 - visible_lines;
        }
        self.clamp_scroll();
    }

    fn scroll_by(&mut self, delta: i32) {
        if delta < 0 {
            self.scroll_line = self
                .scroll_line
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.scroll_line = self.scroll_line.saturating_add(delta as usize);
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let max_scroll = self
            .buffer
            .len_lines()
            .saturating_sub(self.visible_line_count());
        self.scroll_line = self.scroll_line.min(max_scroll);
    }

    fn line_content_len(&self, line_index: usize) -> usize {
        let line = self.buffer.line(line_index);
        let mut length = line.len_chars();
        while length > 0 && matches!(line.char(length - 1), '\n' | '\r') {
            length -= 1;
        }
        length
    }

    fn move_cursor_to_position(&mut self, position: PhysicalPosition<f64>, extend: bool) {
        let visible_line = ((position.y as f32 - 10.0) / LINE_HEIGHT).floor().max(0.0) as usize;
        let line_index = (self.scroll_line + visible_line).min(self.buffer.len_lines() - 1);
        let line_start = self.buffer.line_to_char(line_index);
        let line = self.buffer.line(line_index).to_string();
        let target_column = ((position.x as f32 - GUTTER_WIDTH - 15.0) / CHARACTER_WIDTH)
            .round()
            .max(0.0) as usize;
        let column_index = char_index_at_column(&line, target_column);

        if extend {
            self.selection_anchor.get_or_insert(self.cursor_char_idx);
        } else {
            self.selection_anchor = None;
        }
        self.cursor_char_idx = (line_start + column_index).min(self.buffer.len_chars());
    }

    fn mark_modified(&mut self, changed: bool) {
        if changed {
            self.modified = self.buffer != self.saved_buffer;
            self.update_title();
            self.window.request_redraw();
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        let total_lines = self.buffer.len_lines();
        let content_height = (self.config.height as f32 - 24.0).max(1.0);
        let gutter_text: String = (1..=total_lines).map(|n| format!("{:>3}\n", n)).collect();

        // 1. Buffer do Gutter (Numeração)
        let mut gutter_buffer =
            glyphon::Buffer::new(&mut self.font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        gutter_buffer.set_size(&mut self.font_system, GUTTER_WIDTH, content_height);
        gutter_buffer.set_text(
            &mut self.font_system,
            &gutter_text,
            Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
        );
        gutter_buffer.shape_until_scroll(&mut self.font_system);

        // 2. Buffer de Código com Highlighting
        let mut code_buffer =
            glyphon::Buffer::new(&mut self.font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        let content_width = (self.config.width as f32 - GUTTER_WIDTH - 20.0).max(1.0);
        code_buffer.set_size(&mut self.font_system, content_width, content_height);
        code_buffer.set_wrap(&mut self.font_system, Wrap::None);

        let content = self.buffer.to_string();
        let spans = self.highlighter.highlight(&content);

        gutter_buffer.set_scroll(self.scroll_line as i32);
        code_buffer.set_scroll(self.scroll_line as i32);

        let selection_bytes = self.selection_range().map(|range| {
            (
                self.buffer.char_to_byte(range.start),
                self.buffer.char_to_byte(range.end),
            )
        });
        let mut boundaries = vec![0, content.len()];
        for span in &spans {
            boundaries.push(span.start_byte);
            boundaries.push(span.end_byte);
        }
        if let Some((start, end)) = selection_bytes {
            boundaries.push(start);
            boundaries.push(end);
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut rich_text: Vec<(&str, Attrs)> = Vec::new();
        for pair in boundaries.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if start == end || !content.is_char_boundary(start) || !content.is_char_boundary(end) {
                continue;
            }
            let selected = selection_bytes.is_some_and(|(selection_start, selection_end)| {
                start >= selection_start && end <= selection_end
            });
            let color = if selected {
                SELECTION_TEXT_COLOR
            } else {
                spans
                    .iter()
                    .find(|span| start >= span.start_byte && end <= span.end_byte)
                    .map_or(DEFAULT_TEXT_COLOR, |span| span.color)
            };
            rich_text.push((
                &content[start..end],
                Attrs::new().family(Family::Monospace).color(color),
            ));
        }

        code_buffer.set_rich_text(&mut self.font_system, rich_text, Shaping::Advanced);
        code_buffer.shape_until_scroll(&mut self.font_system);

        let cursor_line = self.buffer.char_to_line(self.cursor_char_idx);
        let cursor_column = self
            .buffer
            .line(self.buffer.char_to_line(self.cursor_char_idx))
            .chars()
            .take(self.cursor_char_idx - self.buffer.line_to_char(cursor_line))
            .map(|character| match character {
                '\t' => INDENT.len(),
                '\n' | '\r' => 0,
                _ => 1,
            })
            .sum::<usize>();
        let cursor_visible = cursor_line >= self.scroll_line
            && cursor_line < self.scroll_line + self.visible_line_count();
        let cursor_screen_line = cursor_line.saturating_sub(self.scroll_line);
        let mut cursor_buffer =
            glyphon::Buffer::new(&mut self.font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));
        cursor_buffer.set_size(&mut self.font_system, CHARACTER_WIDTH, LINE_HEIGHT);
        cursor_buffer.set_text(
            &mut self.font_system,
            if cursor_visible { "│" } else { "" },
            Attrs::new()
                .family(Family::Monospace)
                .color(SELECTION_TEXT_COLOR),
            Shaping::Advanced,
        );
        cursor_buffer.shape_until_scroll(&mut self.font_system);
        let status_text = format!(
            "Ln {}, Col {}    Rust | UTF-8 | {}",
            cursor_line + 1,
            cursor_column + 1,
            if self.modified { "Modified" } else { "Saved" }
        );
        let mut status_buffer =
            glyphon::Buffer::new(&mut self.font_system, Metrics::new(12.0, 20.0));
        status_buffer.set_size(&mut self.font_system, self.config.width as f32, 24.0);
        status_buffer.set_text(
            &mut self.font_system,
            &status_text,
            Attrs::new()
                .family(Family::Monospace)
                .color(Color::rgb(154, 168, 184)),
            Shaping::Advanced,
        );
        status_buffer.shape_until_scroll(&mut self.font_system);

        self.text_renderer
            .prepare(
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
                            bottom: content_height as i32,
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
                            bottom: content_height as i32,
                        },
                        default_color: DEFAULT_TEXT_COLOR,
                    },
                    TextArea {
                        buffer: &cursor_buffer,
                        left: GUTTER_WIDTH + 15.0 + cursor_column as f32 * CHARACTER_WIDTH,
                        top: 10.0 + cursor_screen_line as f32 * LINE_HEIGHT,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: (GUTTER_WIDTH + 15.0) as i32,
                            top: 0,
                            right: self.config.width as i32,
                            bottom: content_height as i32,
                        },
                        default_color: SELECTION_TEXT_COLOR,
                    },
                    TextArea {
                        buffer: &status_buffer,
                        left: 10.0,
                        top: content_height,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: content_height as i32,
                            right: self.config.width as i32,
                            bottom: self.config.height as i32,
                        },
                        default_color: Color::rgb(154, 168, 184),
                    },
                ],
                &mut self.swash_cache,
            )
            .unwrap();

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

            self.text_renderer
                .render(&self.text_atlas, &mut render_pass)
                .unwrap();
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.text_atlas.trim();

        Ok(())
    }
}

fn move_word_cursor(buffer: &Rope, cursor: usize, direction: i8) -> usize {
    let mut target = cursor.min(buffer.len_chars());
    if direction < 0 {
        while target > 0 && buffer.char(target - 1).is_whitespace() {
            target -= 1;
        }
        if target > 0 {
            if is_word_character(buffer.char(target - 1)) {
                while target > 0 && is_word_character(buffer.char(target - 1)) {
                    target -= 1;
                }
            } else {
                target -= 1;
            }
        }
    } else {
        while target < buffer.len_chars() && buffer.char(target).is_whitespace() {
            target += 1;
        }
        if target < buffer.len_chars() {
            if is_word_character(buffer.char(target)) {
                while target < buffer.len_chars() && is_word_character(buffer.char(target)) {
                    target += 1;
                }
            } else {
                target += 1;
            }
        }
    }
    target
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn char_index_at_column(line: &str, target_column: usize) -> usize {
    let mut display_column = 0;
    for (char_index, character) in line.chars().enumerate() {
        let width = match character {
            '\t' => INDENT.len(),
            '\n' | '\r' => return char_index,
            _ => 1,
        };
        if (target_column as f32) < display_column as f32 + width as f32 / 2.0 {
            return char_index;
        }
        display_column += width;
    }
    line.trim_end_matches(['\n', '\r']).chars().count()
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
            "// VS Rustium - Tree-sitter Syntax Highlighting\n\
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
            .with_title("VS Rustium")
            .with_inner_size(winit::dpi::LogicalSize::new(1024, 768))
            .build(&event_loop)
            .unwrap(),
    );

    let mut state = pollster::block_on(EditorState::new(window.clone(), initial_text, file_path));

    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent {
                window_id,
                ref event,
            } if window_id == state.window.id() => match event {
                WindowEvent::CloseRequested => target.exit(),
                WindowEvent::Resized(physical_size) => {
                    state.resize(*physical_size);
                    state.window.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    state.handle_key(event);
                    state.window.request_redraw();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    state.cursor_position = *position;
                    if state.dragging_selection {
                        state.move_cursor_to_position(*position, true);
                        state.window.request_redraw();
                    }
                }
                WindowEvent::MouseInput {
                    state: button_state,
                    button: MouseButton::Left,
                    ..
                } => {
                    if *button_state == ElementState::Pressed {
                        state.move_cursor_to_position(state.cursor_position, false);
                        state.selection_anchor = Some(state.cursor_char_idx);
                        state.dragging_selection = true;
                    } else {
                        state.dragging_selection = false;
                    }
                    state.window.request_redraw();
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let scroll_delta = match delta {
                        MouseScrollDelta::LineDelta(_, lines) => (-lines * 3.0).round() as i32,
                        MouseScrollDelta::PixelDelta(position) => {
                            (-position.y / LINE_HEIGHT as f64).round() as i32
                        }
                    };
                    state.scroll_by(scroll_delta);
                    state.window.request_redraw();
                }
                WindowEvent::ModifiersChanged(modifiers) => {
                    state.control_pressed = modifiers.state().control_key();
                    state.shift_pressed = modifiers.state().shift_key();
                }

                WindowEvent::RedrawRequested => match state.render() {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => state.resize(state.window.inner_size()),
                    Err(wgpu::SurfaceError::OutOfMemory) => target.exit(),
                    Err(e) => eprintln!("Erro ao renderizar: {:?}", e),
                },
                _ => {}
            },
            _ => {}
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{char_index_at_column, move_word_cursor, EditHistory};
    use ropey::Rope;

    #[test]
    fn mouse_columns_respect_tabs_and_unicode_characters() {
        let line = "a\té\n";

        assert_eq!(char_index_at_column(line, 0), 0);
        assert_eq!(char_index_at_column(line, 1), 1);
        assert_eq!(char_index_at_column(line, 5), 2);
        assert_eq!(char_index_at_column(line, 6), 3);
    }

    #[test]
    fn word_navigation_skips_whitespace_and_unicode_identifiers() {
        let buffer = Rope::from_str("hello  café");

        assert_eq!(move_word_cursor(&buffer, 0, 1), 5);
        assert_eq!(move_word_cursor(&buffer, 5, 1), 11);
        assert_eq!(move_word_cursor(&buffer, 11, -1), 7);
        assert_eq!(move_word_cursor(&buffer, 7, -1), 0);
    }

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
