// =============================================================================
// PR0 图标 spike 测试（存档，仅供参考，不参与编译）
//
// 这段代码曾位于 `pc/src-tauri/src/services/fence_fs/icon.rs` 末尾（`mod spike`），
// 收尾清理时从生产代码中移出，内容原样保留。结论与运行方式见同目录 `spike-icons.md`。
//
// 复现时需要放回 crate 内运行：把下面的 `mod spike { ... }` 粘回 `icon.rs` 末尾
// （它通过 `use super::*` 使用 icon.rs 的私有项 `RawImage` / `bgra_to_rgba` /
// `strip_small_icon_frame` / `encode_png`，并通过 `super::super::` 访问 `fence_fs` 的
// `sta::icon_worker`、`shell::extract_image`、`launch_item`），然后在 `pc/src-tauri` 下运行
// `cargo test --lib spike_icons -- --ignored --nocapture` 等命令。所有用例都是 `#[ignore]`，
// 只读取图标、在临时目录生成样本，不改动任何真实文件。
// =============================================================================

/// PR0 图标 spike：`cargo test spike_icons -- --ignored --nocapture`，输出目录由环境变量
/// `FENCE_SPIKE_OUT` 指定（默认系统临时目录下的 `fence-icon-spike`）。只读取图标，不改动任何文件。
#[cfg(all(test, windows))]
mod spike {
    use super::*;
    use windows::Win32::UI::Shell::{
        SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_SCALEUP,
    };

    struct Stats {
        transparent: usize,
        opaque: usize,
        partial: usize,
        straight_evidence: usize,
        zero_alpha_colored: usize,
        alpha254: usize,
        max_alpha: u8,
        bbox: Option<(u32, u32, u32, u32)>,
    }

    fn stats(img: &super::RawImage) -> Stats {
        let mut s = Stats {
            transparent: 0,
            opaque: 0,
            partial: 0,
            straight_evidence: 0,
            zero_alpha_colored: 0,
            alpha254: 0,
            max_alpha: 0,
            bbox: None,
        };
        let (pixels, _) = img.bgra.as_chunks::<4>();
        s.max_alpha = pixels.iter().map(|p| p[3]).max().unwrap_or(0);
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (i, p) in pixels.iter().enumerate() {
            let a = p[3];
            match a {
                0 => s.transparent += 1,
                255 => s.opaque += 1,
                _ => s.partial += 1,
            }
            // 只统计半透明像素：预乘位图里 0 < a < 255 时必有 max(rgb) <= a
            if a > 0 && a < 255 && p[0].max(p[1]).max(p[2]) > a {
                s.straight_evidence += 1;
            }
            if a == 0 && p[0].max(p[1]).max(p[2]) > 0 {
                s.zero_alpha_colored += 1;
            }
            if a == 254 {
                s.alpha254 += 1;
            }
            if a > 16 {
                let (x, y) = (i as u32 % img.width, i as u32 / img.width);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        if x0 != u32::MAX {
            s.bbox = Some((x0, y0, x1, y1));
        }
        s
    }

    /// 把 RGBA 图标以直通 alpha 合成到纯色背景上，写进画布 (ox, oy)
    #[allow(clippy::too_many_arguments)]
    fn blit(
        canvas: &mut [u8],
        cw: u32,
        ox: u32,
        oy: u32,
        w: u32,
        h: u32,
        rgba: &[u8],
        bg: [u8; 3],
    ) {
        for y in 0..h {
            for x in 0..w {
                let s = ((y * w + x) * 4) as usize;
                let a = u32::from(rgba[s + 3]);
                let d = (((oy + y) * cw + ox + x) * 4) as usize;
                for c in 0..3 {
                    let v = (u32::from(rgba[s + c]) * a + u32::from(bg[c]) * (255 - a)) / 255;
                    canvas[d + c] = v as u8;
                }
                canvas[d + 3] = 255;
            }
        }
    }

    /// 用 IShellLinkW 创建快捷方式（在 STA 线程上调用）
    fn create_shortcut(lnk: &Path, target: &Path, icon: Option<&Path>) -> Result<(), String> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            link.SetPath(&HSTRING::from(target))
                .map_err(|e| e.to_string())?;
            if let Some(icon) = icon {
                link.SetIconLocation(&HSTRING::from(icon), 0)
                    .map_err(|e| e.to_string())?;
            }
            let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
            file.Save(&HSTRING::from(lnk), true)
                .map_err(|e| e.to_string())
        }
    }

    /// 只含一张 `size` 像素图（PNG 压缩）的 .ico：橙色圆 + 抗锯齿半透明边缘，模拟只有小图标的老程序
    fn small_ico(size: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((size * size * 4) as usize);
        let c = (size as f32 - 1.0) / 2.0;
        let radius = size as f32 / 2.0 - 1.0;
        for y in 0..size {
            for x in 0..size {
                let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
                let a = ((radius - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
                let ring = d > radius - 2.5;
                let (r, g, b) = if ring { (120, 53, 15) } else { (245, 158, 11) };
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
        let png = encode_png(size, size, &rgba).unwrap();
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        ico.extend_from_slice(&[size as u8, size as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&png);
        ico
    }

    /// 在 `base/input` 下准备样本：快捷方式、.url、普通文件、图片、开始菜单快捷方式的副本
    fn prepare_inputs(base: &Path) -> Vec<PathBuf> {
        let windir = PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into()));
        let input = base.join("input");
        let _ = fs::remove_dir_all(&input);
        fs::create_dir_all(&input).unwrap();
        let worker = super::super::sta::icon_worker().unwrap();
        // 小图标素材放在 input 之外，避免自身也被当作样本
        let assets = base.join("assets");
        fs::create_dir_all(&assets).unwrap();
        let ico16 = assets.join("small16.ico");
        let ico32 = assets.join("small32.ico");
        fs::write(&ico16, small_ico(16)).unwrap();
        fs::write(&ico32, small_ico(32)).unwrap();
        fs::copy(&ico32, input.join("only32.ico")).unwrap();
        for (name, target, icon) in [
            (
                "Notepad.lnk",
                windir.join("System32").join("notepad.exe"),
                None,
            ),
            (
                "Broken.lnk",
                PathBuf::from(r"C:\__fence_spike_missing__\app.exe"),
                None,
            ),
            ("FolderShortcut.lnk", windir.clone(), None),
            (
                "OldApp16.lnk",
                windir.join("System32").join("notepad.exe"),
                Some(ico16.clone()),
            ),
            (
                "OldApp32.lnk",
                windir.join("System32").join("notepad.exe"),
                Some(ico32.clone()),
            ),
        ] {
            let lnk = input.join(name);
            worker
                .run(move || create_shortcut(&lnk, &target, icon.as_deref()))
                .unwrap()
                .unwrap();
        }
        fs::write(
            input.join("Example.url"),
            "[InternetShortcut]\r\nURL=https://example.com/\r\n",
        )
        .unwrap();
        fs::write(input.join("a.txt"), "hello").unwrap();
        for name in ["a.docx", "a.zip", "a.pdf", "a.unknownext"] {
            fs::write(input.join(name), b"").unwrap();
        }
        let wallpaper = windir
            .join("Web")
            .join("Wallpaper")
            .join("Windows")
            .join("img0.jpg");
        if wallpaper.is_file() {
            fs::copy(&wallpaper, input.join("photo.jpg")).unwrap();
        }
        // 开始菜单快捷方式：只复制到临时目录
        if let Some(program_data) = std::env::var_os("ProgramData") {
            let start_menu =
                PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs");
            let mut found: Vec<PathBuf> = Vec::new();
            let mut stack = vec![start_menu];
            while let Some(dir) = stack.pop() {
                for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
                        found.push(p);
                    }
                }
            }
            found.sort();
            for p in found.into_iter().take(5) {
                let name = format!("SM_{}", p.file_name().unwrap().to_string_lossy());
                let _ = fs::copy(&p, input.join(name));
            }
        }
        let mut list: Vec<PathBuf> = fs::read_dir(&input)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        list.sort();
        list
    }

    /// 正方形 RGBA 最近邻缩放（仅用于 spike 拼图展示）
    fn nearest_scale(rgba: &[u8], from: u32, to: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((to * to * 4) as usize);
        for y in 0..to {
            for x in 0..to {
                let (sx, sy) = (x * from / to, y * from / to);
                let i = ((sy * from + sx) * 4) as usize;
                out.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        out
    }

    fn samples(base: &Path) -> Vec<(String, PathBuf)> {
        let windir = PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into()));
        let mut list: Vec<(String, PathBuf)> = vec![
            (
                "notepad.exe".into(),
                windir.join("System32").join("notepad.exe"),
            ),
            ("cmd.exe".into(), windir.join("System32").join("cmd.exe")),
        ];
        for p in prepare_inputs(base) {
            list.push((p.file_name().unwrap().to_string_lossy().into_owned(), p));
        }
        // 额外的 exe（例如只有小图标的老程序），以 ; 分隔
        if let Ok(exes) = std::env::var("FENCE_SPIKE_EXES") {
            for exe in exes.split(';').filter(|s| !s.is_empty()) {
                let p = PathBuf::from(exe);
                list.push((p.file_name().unwrap().to_string_lossy().into_owned(), p));
            }
        }
        list.push(("C:\\Windows (folder)".into(), windir.clone()));
        let empty = base.join("empty-folder");
        fs::create_dir_all(&empty).unwrap();
        list.push(("empty-folder".into(), empty));
        list
    }

    #[test]
    #[ignore]
    fn spike_icons() {
        let out = std::env::var_os("FENCE_SPIKE_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("fence-icon-spike"));
        fs::create_dir_all(&out).unwrap();
        let list = samples(&out);
        let sizes = [48u32, 64, 96];
        // 最后一个变体是实际实现的效果：iconOnly + 去掉小图标边框
        let variants: [(&str, SIIGBF, bool); 5] = [
            ("iconOnly", SIIGBF_ICONONLY, false),
            (
                "iconOnly+bigger",
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
                false,
            ),
            ("iconOnly+scaleUp", SIIGBF_ICONONLY | SIIGBF_SCALEUP, false),
            ("default(thumb)", SIIGBF_RESIZETOFIT, false),
            ("final", SIIGBF_ICONONLY, true),
        ];
        let worker = super::super::sta::icon_worker().unwrap();
        for (vname, flags, strip) in variants {
            // 每格 96 + 8，背景：白 / 深灰（接近收纳箱默认底色）/ 棋盘取中灰
            let cell = 104u32;
            let backgrounds: [[u8; 3]; 3] = [[255, 255, 255], [32, 32, 32], [128, 128, 128]];
            let cols = sizes.len() as u32 * backgrounds.len() as u32;
            let (cw, ch) = (cols * cell, list.len() as u32 * cell);
            let mut canvas = vec![200u8; (cw * ch * 4) as usize];
            println!("\n=== {vname} ===");
            for (row, (label, path)) in list.iter().enumerate() {
                for (si, size) in sizes.iter().enumerate() {
                    let p = path.clone();
                    let size = *size;
                    let result = worker
                        .run(move || super::super::shell::extract_image(&p, size, flags))
                        .unwrap();
                    match result {
                        Ok(img) => {
                            let img = if strip {
                                strip_small_icon_frame(&img).unwrap_or(img)
                            } else {
                                img
                            };
                            let s = stats(&img);
                            let rgba = bgra_to_rgba(&img.bgra);
                            println!(
                                "{label:<34} req {size:>3} → {:>3}x{:<3} opaque {:>5} partial {:>4} (a=254 {:>4}, max a {:>3}) transparent {:>5} (colored {:>4}) straight-evidence {:>4} bbox {:?}",
                                img.width, img.height, s.opaque, s.partial, s.alpha254, s.max_alpha, s.transparent, s.zero_alpha_colored, s.straight_evidence, s.bbox
                            );
                            if img.width <= 96 && img.height <= 96 {
                                for (bi, bg) in backgrounds.iter().enumerate() {
                                    let col = (si * backgrounds.len() + bi) as u32;
                                    // 不足请求尺寸的（去边框后的小图标）按最近邻放大到请求尺寸，模拟 CSS 固定宽高
                                    let (w, h, pixels) = if strip && img.width < size {
                                        (size, size, nearest_scale(&rgba, img.width, size))
                                    } else {
                                        (img.width, img.height, rgba.clone())
                                    };
                                    blit(
                                        &mut canvas,
                                        cw,
                                        col * cell + 4,
                                        row as u32 * cell + 4,
                                        w,
                                        h,
                                        &pixels,
                                        *bg,
                                    );
                                }
                            }
                            if size == 96 {
                                let safe: String = label
                                    .chars()
                                    .map(|c| {
                                        if c.is_alphanumeric() || c == '.' {
                                            c
                                        } else {
                                            '_'
                                        }
                                    })
                                    .collect();
                                let png = encode_png(img.width, img.height, &rgba).unwrap();
                                fs::write(out.join(format!("{vname}-{safe}-{size}.png")), png)
                                    .unwrap();
                            }
                        }
                        Err(e) => println!("{label:<40} req {size:>3} → 失败: {e}"),
                    }
                }
            }
            let png = encode_png(cw, ch, &canvas).unwrap();
            fs::write(out.join(format!("sheet-{vname}.png")), png).unwrap();
        }
        println!("\n输出目录: {}", out.display());
    }

    /// 启动目标失效的快捷方式：SEE_MASK_FLAG_NO_UI 下应直接返回错误、不弹"快捷方式存在问题"对话框
    #[test]
    #[ignore]
    fn spike_launch_broken_shortcut() {
        let out = std::env::temp_dir().join("fence-launch-spike");
        let _ = fs::remove_dir_all(&out);
        fs::create_dir_all(&out).unwrap();
        let lnk = out.join("Broken.lnk");
        let lnk2 = lnk.clone();
        super::super::sta::icon_worker()
            .unwrap()
            .run(move || {
                create_shortcut(
                    &lnk2,
                    Path::new(r"C:\__fence_spike_missing__\app.exe"),
                    None,
                )
            })
            .unwrap()
            .unwrap();
        let started = std::time::Instant::now();
        let result = super::super::launch_item(&lnk.to_string_lossy());
        println!("耗时 {:?}，结果 {:?}", started.elapsed(), result);
        let _ = fs::remove_dir_all(&out);
        assert!(result.is_err());
    }

    /// 打印某个文件按指定尺寸提取后若干行的像素（FENCE_SPIKE_DUMP=路径|尺寸）
    #[test]
    #[ignore]
    fn spike_dump_pixels() {
        let spec = std::env::var("FENCE_SPIKE_DUMP").expect("FENCE_SPIKE_DUMP=路径|尺寸");
        let (path, size) = spec.split_once('|').unwrap();
        let path = PathBuf::from(path);
        let size: u32 = size.parse().unwrap();
        let worker = super::super::sta::icon_worker().unwrap();
        let img = worker
            .run(move || super::super::shell::extract_image(&path, size, SIIGBF_ICONONLY))
            .unwrap()
            .unwrap();
        let w = img.width as usize;
        let rows = [0usize, 1, 2, 3, 10, 20, img.height as usize / 2];
        for y in rows {
            let line: Vec<String> = (0..w.min(40))
                .map(|x| {
                    let p = &img.bgra[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    format!("{:02x}{:02x}{:02x}/{:02x}", p[2], p[1], p[0], p[3])
                })
                .collect();
            println!("y={y:>3}: {}", line.join(" "));
        }
    }

    /// 扫描目录里的 exe，找出"按 256 请求时图标内容很小"的老程序（只有 32/48 px 资源）
    #[test]
    #[ignore]
    fn spike_find_small_icon_exes() {
        let dirs: Vec<PathBuf> = std::env::var("FENCE_SPIKE_SCAN")
            .unwrap_or_else(|_| "C:\\Windows\\System32;C:\\Windows\\SysWOW64".into())
            .split(';')
            .map(PathBuf::from)
            .collect();
        let threshold: u32 = std::env::var("FENCE_SPIKE_BBOX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(48);
        let worker = super::super::sta::icon_worker().unwrap();
        let mut found = 0;
        let mut scanned = 0;
        // (目录, 剩余递归深度)
        let mut stack: Vec<(PathBuf, u32)> = dirs.into_iter().map(|d| (d, 3)).collect();
        while let Some((dir, depth)) = stack.pop() {
            let Ok(read) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if depth > 0 {
                        stack.push((path, depth - 1));
                    }
                    continue;
                }
                if !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                {
                    continue;
                }
                scanned += 1;
                let p = path.clone();
                // 带 BIGGERSIZEOK 时 Shell 不再自己拉伸，返回尺寸小于请求尺寸即说明程序只有小图标资源
                let bigger = std::env::var_os("FENCE_SPIKE_SCAN_BIGGER").is_some();
                let flags = if bigger {
                    SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK
                } else {
                    SIIGBF_ICONONLY
                };
                let Ok(Ok(img)) =
                    worker.run(move || super::super::shell::extract_image(&p, 256, flags))
                else {
                    continue;
                };
                if bigger && img.width < 256 {
                    println!("{} → 返回 {}x{}", path.display(), img.width, img.height);
                    found += 1;
                    if found >= 15 {
                        println!("扫描 {scanned} 个 exe");
                        return;
                    }
                    continue;
                }
                if let Some((x0, y0, x1, y1)) = stats(&img).bbox {
                    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
                    if w <= threshold && h <= threshold {
                        println!(
                            "{} → {}x{} 内容 {}x{}",
                            path.display(),
                            img.width,
                            img.height,
                            w,
                            h
                        );
                        found += 1;
                        if found >= 15 {
                            println!("扫描 {scanned} 个 exe");
                            return;
                        }
                    }
                }
            }
        }
        println!("扫描 {scanned} 个 exe，命中 {found} 个");
    }
}
