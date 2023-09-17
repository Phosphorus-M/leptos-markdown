use leptos::html::AnyElement;
use leptos::*;

use core::ops::Range;

use katex;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use web_sys::MouseEvent;

use pulldown_cmark_wikilink::{
    Alignment, CodeBlockKind, Event, HeadingLevel, MathMode, Tag, TagEnd,
};

use super::{LinkDescription, MarkdownMouseEvent};
use crate::utils::{as_closing_tag, Callback, HtmlCallback};

type Html = HtmlElement<AnyElement>;

pub fn make_callback(
    context: &RenderContext,
    position: Range<usize>,
) -> impl Fn(MouseEvent) + 'static {
    let onclick = context.onclick.clone();
    move |x| {
        let click_event = MarkdownMouseEvent {
            mouse_event: x,
            position: position.clone(),
        };
        onclick.call(click_event)
    }
}

/// all the context needed to render markdown:
pub struct RenderContext {
    /// syntax used for syntax highlighting
    syntax_set: SyntaxSet,

    /// theme used for syntax highlighting
    theme: Theme,

    /// callback to add interactivity to the rendered markdown
    onclick: Callback<MarkdownMouseEvent>,

    /// callback used to render links
    render_links: Option<HtmlCallback<LinkDescription>>,
}

impl RenderContext {
    pub fn new(
        theme_name: Option<String>,
        onclick: Option<Callback<MarkdownMouseEvent>>,
        render_links: Option<HtmlCallback<LinkDescription>>,
    ) -> Self {
        let theme_set = ThemeSet::load_defaults();
        let theme_name = theme_name.unwrap_or("base16-ocean.light".to_string());
        let theme = theme_set
            .themes
            .get(&theme_name)
            .expect("unknown theme")
            .clone();

        let syntax_set = SyntaxSet::load_defaults_newlines();

        RenderContext {
            syntax_set,
            theme,
            onclick: onclick.unwrap_or(Callback::new(|_| ())),
            render_links,
        }
    }
}

pub struct HtmlError(String);

impl HtmlError {
    fn err<T>(message: &str) -> Result<T, Self> {
        Err(HtmlError(message.to_string()))
    }
}

impl ToString for HtmlError {
    fn to_string(&self) -> String {
        self.0.to_owned()
    }
}

use Event::*;

pub struct Renderer<'a, 'c, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    context: &'a RenderContext,
    stream: &'c mut I,
    // TODO: Vec<Alignment> to &[Alignment] to avoid cloning.
    // But it requires to provide the right lifetime
    column_alignment: Option<Vec<Alignment>>,
    cell_index: usize,
    end_tag: Option<TagEnd>,
}

impl<'a, 'c, I> Iterator for Renderer<'a, 'c, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = Html;

    fn next(&mut self) -> Option<Self::Item> {
        let (item, range) = self.stream.next()?;
        let range = range.clone();

        let rendered = match item {
            Start(t) => self.render_tag(t, range),
            End(end) => {
                // check if the closing tag is the tag that was open
                // when this renderer was created
                match self.end_tag {
                    Some(t) if t == end => return None,
                    Some(_) => panic!("wrong closing tag"),
                    None => panic!("didn't expect a closing tag"),
                }
            }
            Text(s) => Ok(render_text(self.context, &s, range)),
            Code(s) => Ok(render_code(self.context, &s, range)),
            Html(s) => Ok(render_html(self.context, &s, range)),
            FootnoteReference(_) => HtmlError::err("do not support footnote refs yet"),
            SoftBreak => Ok(self.next()?),
            HardBreak => Ok(view! {<br/>}.into_any()),
            Rule => Ok(render_rule(self.context, range)),
            TaskListMarker(m) => Ok(render_tasklist_marker(self.context, m, range)),
            Math(disp, content) => render_maths(self.context, &content, &disp, range),
        };

        Some(rendered.unwrap_or_else(|e| {
            view! {
            <span class="error" style="border: 1px solid red">
                {e.to_string()}
                <br/>
            </span>
            }
            .into_any()
        }))
    }
}

impl<'a, 'c, I> Renderer<'a, 'c, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    pub fn new(context: &'a RenderContext, events: &'c mut I) -> Self {
        Self {
            context,
            stream: events,
            column_alignment: None,
            cell_index: 0,
            end_tag: None,
        }
    }

    fn children(&mut self, tag: Tag<'a>) -> View {
        let sub_renderer = Renderer {
            context: self.context,
            stream: self.stream,
            column_alignment: self.column_alignment.clone(),
            cell_index: 0,
            end_tag: Some(as_closing_tag(&tag)),
        };
        sub_renderer.collect_view()
    }

    fn children_text(&mut self, tag: Tag<'a>) -> Option<String> {
        let text = match self.stream.next() {
            Some((Event::Text(s), _)) => Some(s.to_string()),
            None => None,
            _ => panic!("expected string event, got something else"),
        };

        let end_tag = &self
            .stream
            .next()
            .expect("this event should be the closing tag")
            .0;
        assert!(end_tag == &Event::End(as_closing_tag(&tag)));

        text
    }

    fn render_tag(&mut self, tag: Tag<'a>, range: Range<usize>) -> Result<Html, HtmlError> {
        Ok(match tag.clone() {
            Tag::Paragraph => view! {<p>{self.children(tag)}</p>}.into_any(),
            Tag::Heading { level, .. } => render_heading(level, self.children(tag)),
            Tag::BlockQuote => view! {
                <blockquote>
                    {self.children(tag)}
                </blockquote>
            }
            .into_any(),
            Tag::CodeBlock(k) => {
                render_code_block(self.context, self.children_text(tag), &k, range)
            }
            Tag::List(Some(n0)) => view! {
            <ol start=n0 as i32>
                {self.children(tag)}
            </ol>}
            .into_any(),
            Tag::List(None) => view! { <ul>{self.children(tag)}</ul>}.into_any(),
            Tag::Item => view! { <li>{self.children(tag)}</li>}.into_any(),
            Tag::Table(align) => {
                self.column_alignment = Some(align);
                view! { <table>{self.children(tag)}</table>}.into_any()
            }
            Tag::TableHead => view! {
                <thead>{self.children(tag)}</thead>
            }
            .into_any(),
            Tag::TableRow => view! {
                <tr>{self.children(tag)}</tr>
            }
            .into_any(),
            Tag::TableCell => {
                let align = self.column_alignment.clone().unwrap()[self.cell_index];
                self.cell_index += 1;
                render_cell(self.children(tag), &align)
            }
            Tag::Emphasis => view! { <i>{self.children(tag)}</i>}.into_any(),
            Tag::Strong => view! { <b>{self.children(tag)}</b>}.into_any(),
            Tag::Strikethrough => view! { <s>{self.children(tag)}</s>}.into_any(),
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let description = LinkDescription {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    content: self.children(tag),
                    link_type,
                    image: true,
                };
                render_link(self.context, description)?
            }
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let description = LinkDescription {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    content: self.children(tag),
                    link_type,
                    image: false,
                };
                render_link(self.context, description)?
            }
            Tag::FootnoteDefinition(_) => return HtmlError::err("footnote: not implemented"),
            Tag::MetadataBlock { .. } => {
                let _ = self.children(tag);
                view! { <div></div>}.into_any()
            }
        })
    }
}

fn render_tasklist_marker(context: &RenderContext, m: bool, position: Range<usize>) -> Html {
    let onclick = context.onclick.clone();
    let callback = move |e: MouseEvent| {
        e.prevent_default();
        e.stop_propagation();
        let click_event = MarkdownMouseEvent {
            mouse_event: e,
            position: position.clone(),
        };
        onclick.call(click_event)
    };
    view! {
         <input type="checkbox" checked=m on:click=callback>
            </input>
    }
    .into_any()
}

fn render_rule(context: &RenderContext, range: Range<usize>) -> Html {
    let callback = make_callback(context, range);
    view! { <hr on:click=callback/>}.into_any()
}

fn render_html(context: &RenderContext, s: &str, range: Range<usize>) -> Html {
    let callback = make_callback(context, range);
    view! {
        <div on:click=callback inner_html={s.to_string()}>
        </div>
    }
    .into_any()
}

fn render_code(context: &RenderContext, s: &str, range: Range<usize>) -> Html {
    let callback = make_callback(context, range);
    view! { <code on:click=callback>{s.to_string()}</code>}.into_any()
}

fn render_text(context: &RenderContext, s: &str, range: Range<usize>) -> Html {
    let callback = make_callback(context, range);
    view! {
        <span on:click=callback>
            {s.to_string()}
        </span>
    }
    .into_any()
}

fn render_code_block(
    context: &RenderContext,
    string_content: Option<String>,
    k: &CodeBlockKind,
    range: Range<usize>,
) -> Html {
    let content = match string_content {
        Some(x) => x,
        None => {
            return view! {
                <code></code>
            }
            .into_any()
        }
    };

    let callback = make_callback(context, range);

    match highlight_code(context, &content, &k) {
        None => view! {
        <code on:click=callback>
            <pre inner_html=content.to_string()></pre>
        </code>
        }
        .into_any(),
        Some(x) => view! {
            <div on:click=callback inner_html=x>
                </div>
        }
        .into_any(),
    }
}

/// `highlight_code(content, ss, ts)` render the content `content`
/// with syntax highlighting
fn highlight_code(context: &RenderContext, content: &str, kind: &CodeBlockKind) -> Option<String> {
    let lang = match kind {
        CodeBlockKind::Fenced(x) => x,
        CodeBlockKind::Indented => return None,
    };
    Some(
        syntect::html::highlighted_html_for_string(
            content,
            &context.syntax_set,
            context.syntax_set.find_syntax_by_token(lang)?,
            &context.theme,
        )
        .ok()?,
    )
}

/// `render_header(d, s)` returns the html corresponding to
/// the string `s` inside a html header with depth `d`
fn render_heading<I: IntoView>(level: HeadingLevel, content: I) -> Html {
    use HeadingLevel::*;
    match level {
        H1 => view! {<h1>{content}</h1>}.into_any(),
        H2 => view! {<h2>{content}</h2>}.into_any(),
        H3 => view! {<h3>{content}</h3>}.into_any(),
        H4 => view! {<h4>{content}</h4>}.into_any(),
        H5 => view! {<h5>{content}</h5>}.into_any(),
        H6 => view! {<h6>{content}</h6>}.into_any(),
    }
}

/// `render_maths(content)` returns a html node
/// with the latex content `content` compiled inside
fn render_maths(
    context: &RenderContext,
    content: &str,
    display_mode: &MathMode,
    range: Range<usize>,
) -> Result<Html, HtmlError> {
    let opts = katex::Opts::builder()
        .display_mode(*display_mode == MathMode::Display)
        .build()
        .unwrap();

    let class_name = match display_mode {
        MathMode::Inline => "math-inline",
        MathMode::Display => "math-flow",
    };

    let callback = make_callback(context, range);

    match katex::render_with_opts(content, opts) {
        Ok(_) => Ok(view! {
            <span inner_html=x class=class_name on:click=callback></span>
        }
        .into_any()),
        Err(_) => HtmlError::err("invalid math"),
    }
}

fn render_link(context: &RenderContext, link: LinkDescription) -> Result<Html, HtmlError> {
    match (&context.render_links, link.image) {
        (Some(f), _) => Ok(f.call(link)),
        (None, false) => Ok(view! {
            <a href={link.url}>
                {link.content}
            </a>
        }
        .into_any()),
        (None, true) => Ok(view! {
            <img src={link.url} alt=link.title/>
        }
        .into_any()),
    }
}

/// `align_string(align)` gives the css string
/// that is used to align text according to `align`
fn align_string(align: &Alignment) -> &'static str {
    match align {
        Alignment::Left => "text-align: left",
        Alignment::Right => "text-align: right",
        Alignment::Center => "text-align: center",
        Alignment::None => "",
    }
}

/// `render_cell(cell, align, context)` renders cell as html,
/// and use `align` to
fn render_cell<'a>(content: View, align: &'a Alignment) -> Html {
    view! {
        <td style={align_string(align)}>
            {content}
        </td>
    }
    .into_any()
}
