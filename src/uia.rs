//! Reading the focused text field through Windows UI Automation.
//! Only the text *before the caret* of the focused, editable, non-password
//! field is read, and only after the user pauses typing.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface};

/// Snapshot of the field the user is typing in.
#[derive(Debug, Clone)]
pub struct FieldContext {
    /// Up to ~3000 chars of the field ending at the caret.
    pub before: String,
    /// Text after the caret, up to ~1000 chars (context for the model).
    pub after: String,
    /// Rest of the current line after the caret (to know if the caret is at the end).
    pub after_on_line: String,
    /// Caret rectangle in physical screen pixels.
    pub caret: RECT,
    pub hwnd: HWND,
    pub exe: String,
    pub title: String,
    /// How the text was obtained (for diagnostics / compatibility notes).
    pub source: &'static str,
}

/// Field text around the caret: `full[..caret]` is before it, `full[caret..]` after.
#[derive(Debug, Clone)]
pub struct FieldText {
    pub full: Vec<char>,
    pub caret: usize,
    pub source: &'static str,
}

/// A paragraph (line) of the field: char offset in `full` and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start: usize,
    pub text: String,
}

/// Split field text into lines (paragraphs), keeping char offsets.
pub fn lines(full: &[char]) -> Vec<Line> {
    let mut out = vec![];
    let mut start = 0;
    for i in 0..=full.len() {
        if i == full.len() || full[i] == '\n' || full[i] == '\r' {
            out.push(Line { start, text: full[start..i].iter().collect() });
            start = i + 1;
        }
    }
    out
}

impl FieldText {
    /// Index of the line containing the caret.
    pub fn caret_line(&self) -> Option<usize> {
        lines(&self.full).iter().position(|l| l.start <= self.caret && self.caret <= l.start + l.text.chars().count())
    }

    /// Where paragraph `para` starts, relative to the caret (negative = before it).
    /// Paragraphs are found by their text, so this survives caret moves and edits
    /// elsewhere. The nearest whole-line match wins.
    pub fn locate(&self, para: &str) -> Option<isize> {
        let para = para.trim_end();
        lines(&self.full)
            .into_iter()
            .filter(|l| l.text.trim_end() == para)
            .map(|l| l.start as isize - self.caret as isize)
            .min_by_key(|d| d.abs())
    }
}

#[derive(Debug)]
pub enum Skip {
    NoFocus,
    Password,
    NotEditable,
    OwnProcess,
    NoTextAccess(String),
}

pub struct Reader {
    uia: IUIAutomation,
}

/// The whole message is context for the model; bounded so huge documents stay cheap.
const CONTEXT_CHARS: i32 = 3000;
const AFTER_CHARS: i32 = 1000;

impl Reader {
    /// Must be called on a thread that has initialized COM (MTA recommended).
    pub fn new() -> windows::core::Result<Self> {
        let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
        Ok(Reader { uia })
    }

    pub fn read_focused(&self) -> Result<FieldContext, Skip> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return Err(Skip::NoFocus);
            }
            let el = self.uia.GetFocusedElement().map_err(|_| Skip::NoFocus)?;
            let pid = el.CurrentProcessId().unwrap_or(0) as u32;
            if pid == GetCurrentProcessId() {
                return Err(Skip::OwnProcess);
            }
            // Never touch password fields.
            if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(true) {
                return Err(Skip::Password);
            }
            let ctype = el.CurrentControlType().unwrap_or(UIA_CustomControlTypeId);
            let value: Option<IUIAutomationValuePattern> = el.GetCurrentPatternAs(UIA_ValuePatternId).ok();
            let read_only = value.as_ref().and_then(|v| v.CurrentIsReadOnly().ok()).map(|b| b.as_bool());
            let editable = match (ctype, read_only) {
                (_, Some(true)) => false,
                (UIA_EditControlTypeId, _) => true,
                // Rich editors (web contenteditable, chat boxes) expose Document/Group + writable Value.
                (_, Some(false)) => true,
                _ => false,
            };
            if !editable {
                return Err(Skip::NotEditable);
            }
            let exe = process_name(pid);
            let title = window_title(hwnd);

            let (before, after, after_on_line, caret, source) = self.read_text(&el)?;
            Ok(FieldContext { before, after, after_on_line, caret, hwnd, exe, title, source })
        }
    }

    /// The focused element and its caret range (TextPattern fields only).
    unsafe fn caret(&self) -> Option<(IUIAutomationElement, IUIAutomationTextRange)> {
        unsafe {
            let el = self.uia.GetFocusedElement().ok()?;
            let tp: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
            let r = match el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
                Ok(tp2) => {
                    let mut active = BOOL(0);
                    tp2.GetCaretRange(&mut active).ok()?
                }
                Err(_) => tp.GetSelection().ok()?.GetElement(0).ok()?,
            };
            Some((el, r))
        }
    }

    /// Snapshot of the field text around the caret (same window as `read_focused`).
    pub fn field_text(&self) -> Option<FieldText> {
        let ctx = self.read_focused().ok()?;
        let mut full: Vec<char> = ctx.before.chars().collect();
        let caret = full.len();
        full.extend(ctx.after.chars());
        Some(FieldText { full, caret, source: ctx.source })
    }

    /// Range from `ds` to `de` characters relative to the caret, verified to read `expect`.
    unsafe fn range_rel(&self, ds: isize, de: isize, expect: &str) -> Option<IUIAutomationTextRange> {
        unsafe {
            let (_, caret) = self.caret()?;
            let r = caret.Clone().ok()?;
            let (ds, de) = (ds as i32, de as i32);
            // Move the endpoint that keeps start <= end valid first.
            if ds < 0 {
                r.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, ds).ok()?;
                r.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, de).ok()?;
            } else {
                r.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, de).ok()?;
                r.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, ds).ok()?;
            }
            let got = r.GetText(-1).ok()?.to_string().replace('\u{a0}', " ");
            crate::engine::dlog(&format!("range_rel {ds}..{de} expect={expect:?} got={got:?}"));
            (got == expect).then_some(r)
        }
    }

    /// On-screen rectangles for spans given relative to the caret, each verified to read
    /// its expected text (empty when it doesn't, or when the field has no positions).
    pub fn rects_rel(&self, spans: &[(isize, isize, String)]) -> Vec<Vec<RECT>> {
        unsafe { spans.iter().map(|(s, e, expect)| self.range_rel(*s, *e, expect).map(|r| all_rects(&r)).unwrap_or_default()).collect() }
    }

    /// Select `ds..de` (relative to the caret), verified to read `expect`, so typing
    /// replaces exactly that text.
    pub fn select_rel(&self, ft: &FieldText, ds: isize, de: isize, expect: &str) -> bool {
        unsafe {
            if let Some(r) = self.range_rel(ds, de, expect) {
                return r.Select().is_ok();
            }
            // Classic Win32 EDIT: select by offset with EM_SETSEL.
            if ft.source == "Win32 Edit" {
                return classic_select(ft, ds, de, expect);
            }
            false
        }
    }

    unsafe fn read_text(&self, el: &IUIAutomationElement) -> Result<(String, String, String, RECT, &'static str), Skip> {
        unsafe {
            let tp: IUIAutomationTextPattern = match el.GetCurrentPatternAs(UIA_TextPatternId) {
                Ok(tp) => tp,
                // Classic Win32 EDIT controls expose only ValuePattern.
                Err(_) => return classic_edit(el).ok_or_else(|| Skip::NoTextAccess("no TextPattern".into())),
            };

            // Caret: TextPattern2 (Chromium, modern controls) or the selection start.
            let mut source = "TextPattern2";
            let caret_range: IUIAutomationTextRange = match el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
                Ok(tp2) => {
                    let mut active = BOOL(0);
                    tp2.GetCaretRange(&mut active).map_err(|e| Skip::NoTextAccess(e.to_string()))?
                }
                Err(_) => {
                    source = "TextPattern";
                    let sel = tp.GetSelection().map_err(|e| Skip::NoTextAccess(e.to_string()))?;
                    if sel.Length().unwrap_or(0) < 1 {
                        return Err(Skip::NoTextAccess("no selection".into()));
                    }
                    sel.GetElement(0).map_err(|e| Skip::NoTextAccess(e.to_string()))?
                }
            };
            // A non-empty selection means the user is selecting, not typing.
            let sel_text = caret_range.GetText(2).map(|b| b.to_string()).unwrap_or_default();
            if !sel_text.is_empty() {
                return Err(Skip::NoTextAccess("selection active".into()));
            }

            // In browsers the DocumentRange can span the whole page. Bound everything
            // to the focused editor so text from other fields is never read.
            let doc = match tp.RangeFromChild(el) {
                Ok(r) => r,
                Err(_) => tp.DocumentRange().map_err(|e| Skip::NoTextAccess(e.to_string()))?,
            };
            let before_r = caret_range.Clone().map_err(|e| Skip::NoTextAccess(e.to_string()))?;
            // Only the tail before the caret is needed.
            let _ = before_r.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -CONTEXT_CHARS);
            if before_r.CompareEndpoints(TextPatternRangeEndpoint_Start, &doc, TextPatternRangeEndpoint_Start).unwrap_or(0) < 0 {
                let _ = before_r.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &doc, TextPatternRangeEndpoint_Start);
            }
            let before = before_r.GetText(CONTEXT_CHARS + 8).map(|b| b.to_string()).unwrap_or_default();

            let after_r = caret_range.Clone().map_err(|e| Skip::NoTextAccess(e.to_string()))?;
            let _ = after_r.MoveEndpointByRange(TextPatternRangeEndpoint_End, &doc, TextPatternRangeEndpoint_End);
            let after = after_r.GetText(AFTER_CHARS).map(|b| b.to_string()).unwrap_or_default();
            // Browsers turn a trailing space into a no-break space.
            let before = before.replace('\u{a0}', " ");
            let after = after.replace('\u{a0}', " ");
            let after_on_line = after.split(['\n', '\r']).next().unwrap_or("").to_string();

            let caret = caret_rect(&caret_range).or_else(gui_caret).or_else(|| element_rect(el)).unwrap_or_default();
            Ok((before, after, after_on_line, caret, source))
        }
    }
}

/// Classic EDIT control: text from ValuePattern, caret offset from EM_GETSEL.
unsafe fn classic_edit(el: &IUIAutomationElement) -> Option<(String, String, String, RECT, &'static str)> {
    unsafe {
        let hwnd = el.CurrentNativeWindowHandle().ok()?;
        if hwnd.0.is_null() {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]).to_lowercase();
        if !class.contains("edit") {
            return None;
        }
        let value: IUIAutomationValuePattern = el.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        let text: Vec<u16> = value.CurrentValue().ok()?.to_string().encode_utf16().collect();
        let mut res = 0usize;
        const EM_GETSEL: u32 = 0x00B0;
        SendMessageTimeoutW(hwnd, EM_GETSEL, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 100, Some(&mut res)).0;
        let (start, end) = ((res & 0xFFFF) as usize, ((res >> 16) & 0xFFFF) as usize);
        if start != end || end > text.len() {
            return None; // selection active, or offsets beyond 64K
        }
        let lo = end.saturating_sub(CONTEXT_CHARS as usize);
        let before = String::from_utf16_lossy(&text[lo..end]);
        let after = String::from_utf16_lossy(&text[end..text.len().min(end + AFTER_CHARS as usize)]);
        let after_on_line = after.split(['\n', '\r']).next().unwrap_or("").to_string();
        let caret = gui_caret().or_else(|| element_rect(el)).unwrap_or_default();
        Some((before, after, after_on_line, caret, "Win32 Edit"))
    }
}

/// All line rectangles of a range (a span can wrap onto two lines).
unsafe fn all_rects(r: &IUIAutomationTextRange) -> Vec<RECT> {
    unsafe {
        let mut out = vec![];
        let Ok(sa) = r.GetBoundingRectangles() else { return out };
        if sa.is_null() {
            return out;
        }
        if let (Ok(lb), Ok(ub)) = (SafeArrayGetLBound(sa, 1), SafeArrayGetUBound(sa, 1)) {
            let n = (ub - lb + 1).max(0) as usize;
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            if n >= 4 && SafeArrayAccessData(sa, &mut p).is_ok() {
                let d = std::slice::from_raw_parts(p as *const f64, n);
                for q in d.chunks_exact(4) {
                    if q[2] > 0.0 && q[3] > 0.0 {
                        out.push(RECT { left: q[0] as i32, top: q[1] as i32, right: (q[0] + q[2]) as i32, bottom: (q[1] + q[3]) as i32 });
                    }
                }
                let _ = SafeArrayUnaccessData(sa);
            }
        }
        let _ = SafeArrayDestroy(sa);
        out
    }
}

/// EM_SETSEL on the focused classic EDIT control, after checking the text there.
unsafe fn classic_select(ft: &FieldText, ds: isize, de: isize, expect: &str) -> bool {
    unsafe {
        let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(0, &mut gti).is_err() || gti.hwndFocus.0.is_null() {
            return false;
        }
        let (start, end) = (ft.caret as isize + ds, ft.caret as isize + de);
        if start < 0 || end as usize > ft.full.len() || ft.full[start as usize..end as usize].iter().collect::<String>() != expect {
            return false;
        }
        let u16_of = |n: usize| ft.full[..n].iter().map(|c| c.len_utf16()).sum::<usize>() as isize;
        const EM_GETSEL: u32 = 0x00B0;
        const EM_SETSEL: u32 = 0x00B1;
        let mut res = 0usize;
        SendMessageTimeoutW(gti.hwndFocus, EM_GETSEL, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 100, Some(&mut res));
        let caret = ((res >> 16) & 0xFFFF) as isize;
        // The snapshot may start mid-text (it's a window around the caret): offset from the caret.
        let base = caret - u16_of(ft.caret);
        let (a, b) = (base + u16_of(start as usize), base + u16_of(end as usize));
        if a < 0 {
            return false;
        }
        SendMessageTimeoutW(gti.hwndFocus, EM_SETSEL, WPARAM(a as usize), LPARAM(b), SMTO_ABORTIFHUNG, 100, None);
        true
    }
}

/// Bounding box of the caret. Degenerate ranges often report no rectangles, so
/// widen to the previous character and use its right edge.
unsafe fn caret_rect(r: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        if let Some(rc) = first_rect(r) {
            return Some(rc);
        }
        let w = r.Clone().ok()?;
        w.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).ok()?;
        let rc = first_rect(&w)?;
        Some(RECT { left: rc.right, right: rc.right + 1, ..rc })
    }
}

unsafe fn first_rect(r: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        let sa = r.GetBoundingRectangles().ok()?;
        if sa.is_null() {
            return None;
        }
        let lb = SafeArrayGetLBound(sa, 1).ok()?;
        let ub = SafeArrayGetUBound(sa, 1).ok()?;
        let n = (ub - lb + 1) as usize;
        let mut out = None;
        if n >= 4 {
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            if SafeArrayAccessData(sa, &mut p).is_ok() {
                let d = std::slice::from_raw_parts(p as *const f64, n);
                // Use the last rectangle: the line the caret is on.
                let i = n - 4;
                let (x, y, w, h) = (d[i], d[i + 1], d[i + 2], d[i + 3]);
                if h > 0.0 {
                    out = Some(RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 });
                    if w > 2.0 {
                        // Caret sits at the end of the last rectangle.
                        out.as_mut().unwrap().left = (x + w) as i32;
                    }
                }
                let _ = SafeArrayUnaccessData(sa);
            }
        }
        let _ = SafeArrayDestroy(sa);
        out
    }
}

/// Win32 caret (classic edit controls) via GetGUIThreadInfo.
fn gui_caret() -> Option<RECT> {
    unsafe {
        let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(0, &mut gti).ok()?;
        if gti.hwndCaret.0.is_null() || gti.rcCaret.bottom <= gti.rcCaret.top {
            return None;
        }
        let mut tl = POINT { x: gti.rcCaret.left, y: gti.rcCaret.top };
        let mut br = POINT { x: gti.rcCaret.right, y: gti.rcCaret.bottom };
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(gti.hwndCaret, &mut tl);
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(gti.hwndCaret, &mut br);
        Some(RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y })
    }
}

unsafe fn element_rect(el: &IUIAutomationElement) -> Option<RECT> {
    unsafe {
        let r = el.CurrentBoundingRectangle().ok()?;
        // Anchor at the bottom-left of the field.
        Some(RECT { left: r.left + 8, top: r.bottom - 24, right: r.left + 9, bottom: r.bottom - 4 })
    }
}

pub fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

pub fn window_title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ft(before: &str, after: &str) -> FieldText {
        let mut full: Vec<char> = before.chars().collect();
        let caret = full.len();
        full.extend(after.chars());
        FieldText { full, caret, source: "test" }
    }

    #[test]
    fn locates_paragraphs_by_text() {
        let f = ft("Hi team,\nthe launch went well\nnext week we fix", " it");
        assert_eq!(f.locate("Hi team,"), Some(-(f.caret as isize)));
        assert_eq!(f.locate("the launch went well"), Some(9 - f.caret as isize));
        assert_eq!(f.locate("next week we fix it"), Some(30 - f.caret as isize));
        assert_eq!(f.locate("not there"), None);
        assert_eq!(lines(&f.full)[f.caret_line().unwrap()].text, "next week we fix it");
    }

    #[test]
    fn nearest_duplicate_wins() {
        let f = ft("ok\nfar away\nok\n", "");
        assert_eq!(f.locate("ok"), Some(12 - f.caret as isize));
    }
}

// Keep the Interface import used (GetCurrentPatternAs is generic over Interface).
#[allow(dead_code)]
fn _assert_interface<T: Interface>() {}
