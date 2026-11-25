// SPDX-License-Identifier: Apache-2.0
use super::*;
use iced::widget::{
    button, checkbox, column, container, image, opaque, pick_list, progress_bar, row, scrollable,
    stack, text, text_editor, text_input, tooltip, Column, Space,
};
use iced::{Alignment, Color, Length};

const ROW_HEIGHT: f32 = 76.;
fn control(label: &str, message: Msg) -> iced::widget::Button<'_, Msg> {
    button(text(label).size(14))
        .padding([9, 14])
        .on_press(message)
        .style(|theme: &Theme, state| {
            let dark = theme.palette().background.r < 0.3;
            let hover = matches!(state, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    if dark {
                        if hover {
                            Color::from_rgb8(53, 55, 52)
                        } else {
                            Color::from_rgb8(41, 42, 41)
                        }
                    } else if hover {
                        Color::from_rgb8(233, 231, 227)
                    } else {
                        Color::from_rgb8(254, 253, 251)
                    }
                    .into(),
                ),
                text_color: theme.palette().text,
                border: iced::Border {
                    radius: 7.into(),
                    width: 1.,
                    color: if dark {
                        Color::from_rgb8(73, 75, 72)
                    } else {
                        Color::from_rgb8(230, 228, 223)
                    },
                },
                ..Default::default()
            }
        })
}
fn heading(label: &str) -> iced::widget::Text<'_> {
    text(label).size(25)
}
fn muted<'a>(label: impl Into<String>) -> iced::widget::Text<'a> {
    text(label.into())
        .size(12)
        .style(|theme: &Theme| text::Style {
            color: Some(if theme.palette().background.r < 0.3 {
                Color::from_rgb8(163, 165, 164)
            } else {
                Color::from_rgb8(119, 122, 124)
            }),
        })
}
fn short(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}
fn empty<'a>(label: &'a str) -> Element<'a, Msg> {
    container(muted(label))
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

impl App {
    fn compact(&self) -> bool {
        self.window_state["width"].as_f64().unwrap_or(1180.) < 960.
    }
    fn short_window(&self) -> bool {
        self.window_state["height"].as_f64().unwrap_or(820.) < 650.
    }
    pub(super) fn shell_view(&self) -> Element<'_, Msg> {
        let navigation = column![
            text("流光").size(29),
            Space::new().height(22),
            button(text("＋ 添加下载").size(15))
                .width(Length::Fill)
                .padding([12, 15])
                .on_press(Msg::Page(Page::Add)),
            Space::new().height(15),
            self.nav("下载中", Page::Active),
            self.nav("历史记录", Page::History),
            Space::new().height(Length::Fill),
            iced::widget::rule::horizontal(1),
            self.nav("设置", Page::Settings),
            self.nav("工具", Page::Tools),
            control("退出", Msg::ConfirmExit),
        ]
        .spacing(8)
        .padding(22)
        .width(188)
        .height(Length::Fill);
        let content = match self.base_page() {
            Page::Active | Page::History => self.tasks_view(),
            Page::Settings => self.settings_view(),
            Page::Tools => self.tools_view(),
            _ => self.tasks_view(),
        };
        let pane: Element<'_, Msg> = if self.is_modal() {
            content
        } else {
            self.feedback_view(content)
        };
        let sidebar = container(navigation).style(|theme: &Theme| {
            let dark = theme.palette().background.r < 0.3;
            container::Style {
                background: Some(
                    if dark {
                        Color::from_rgb8(30, 31, 31)
                    } else {
                        Color::from_rgb8(240, 239, 236)
                    }
                    .into(),
                ),
                ..Default::default()
            }
        });
        let pane = container(pane)
            .padding(if self.compact() { 16 } else { 28 })
            .width(Length::Fill)
            .height(Length::Fill);
        let shell: Element<'_, Msg> = if self.compact() {
            column![
                container(
                    row![
                        text("流光").size(23),
                        Space::new().width(Length::Fill),
                        control("＋ 下载", Msg::Page(Page::Add)),
                        control("任务", Msg::Page(Page::Active)),
                        control("历史", Msg::Page(Page::History)),
                        control("设置", Msg::Page(Page::Settings)),
                        control("工具", Msg::Page(Page::Tools))
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                )
                .padding([10, 16]),
                pane
            ]
            .into()
        } else {
            row![sidebar, pane].into()
        };
        if !self.is_modal() {
            return shell;
        }
        let content = match self.page {
            Page::Add => self.add_view(),
            Page::Advanced => column![
                row![
                    heading("高级设置"),
                    Space::new().width(Length::Fill),
                    control("关闭", Msg::ModalClose)
                ],
                self.advanced_view()
            ]
            .spacing(16)
            .height(Length::Fill)
            .into(),
            Page::Detail => self.detail_view(),
            _ => unreachable!(),
        };
        let content = self.feedback_view(content);
        let width = self.window_state["width"].as_f64().unwrap_or(1180.) as f32;
        let height = self.window_state["height"].as_f64().unwrap_or(820.) as f32;
        let inset = if width < 780. || height < 650. {
            12.
        } else {
            24.
        };
        let modal_width = (if self.page == Page::Advanced {
            1040.0f32
        } else {
            750.0f32
        })
        .min(width - inset * 2.);
        let modal = container(content)
            .padding(if self.compact() { 16 } else { 24 })
            .width(modal_width)
            .height((height - inset * 2.).min(if self.page == Page::Advanced {
                780.
            } else {
                820.
            }))
            .style(|theme: &Theme| {
                let dark = theme.palette().background.r < 0.3;
                container::Style {
                    background: Some(
                        if dark {
                            Color::from_rgb8(41, 42, 41)
                        } else {
                            Color::from_rgb8(254, 253, 251)
                        }
                        .into(),
                    ),
                    border: iced::Border {
                        radius: 11.into(),
                        width: 1.,
                        color: if dark {
                            Color::from_rgb8(73, 75, 72)
                        } else {
                            Color::from_rgb8(210, 208, 204)
                        },
                    },
                    shadow: iced::Shadow {
                        color: Color::from_rgba8(0, 0, 0, 0.2),
                        offset: iced::Vector::new(0., 20.),
                        blur_radius: 48.,
                    },
                    ..Default::default()
                }
            });
        let backdrop = container(opaque(modal))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| container::Style {
                background: Some(Color::from_rgba8(24, 29, 37, 0.22).into()),
                ..Default::default()
            });
        stack![crate::modal::inert(shell), opaque(backdrop)].into()
    }
    fn feedback_view<'a>(&'a self, content: Element<'a, Msg>) -> Element<'a, Msg> {
        let mut pane = column![content].height(Length::Fill).spacing(8);
        if !self.error.is_empty() {
            pane = pane.push(
                container(
                    row![
                        text(&self.error)
                            .size(13)
                            .color(Color::from_rgb8(167, 66, 53))
                            .width(Length::Fill),
                        control("关闭", Msg::Dismiss)
                    ]
                    .spacing(10),
                )
                .padding(10)
                .style(container::rounded_box),
            );
        } else if !self.notice.is_empty() {
            pane = pane.push(
                row![
                    muted(self.notice.clone()).width(Length::Fill),
                    control("关闭", Msg::Dismiss)
                ]
                .spacing(10),
            );
        }
        pane.into()
    }
    fn thumbnail(&self, url: &str) -> Element<'_, Msg> {
        if let Some(handle) = self.images.ready.get(url) {
            image(handle.clone())
                .width(76)
                .height(43)
                .content_fit(iced::ContentFit::Cover)
                .into()
        } else {
            container(text("▷").size(22))
                .center_x(76)
                .center_y(43)
                .style(container::rounded_box)
                .into()
        }
    }
    fn nav(&self, label: &'static str, page: Page) -> Element<'_, Msg> {
        button(text(label).size(15))
            .padding([10, 12])
            .width(Length::Fill)
            .style(if self.base_page() == page {
                button::primary
            } else {
                button::text
            })
            .on_press(Msg::Page(page))
            .into()
    }
    fn text_field<'a>(&'a self, key: &'static str, placeholder: &'a str) -> Element<'a, Msg> {
        text_input(placeholder, self.field(key))
            .id(key)
            .on_input(move |v| Msg::Text(key.into(), v))
            .padding(10)
            .size(14)
            .into()
    }
    fn setting_field<'a>(&'a self, key: &'static str, label: &'a str) -> Element<'a, Msg> {
        column![
            muted(label),
            text_input(label, s(self.active_settings(), key))
                .on_input(move |v| Msg::Setting(key.into(), v.into()))
                .padding(10)
                .size(14)
        ]
        .spacing(6)
        .into()
    }
    fn setting_pick(
        &self,
        key: &'static str,
        label: &'static str,
        options: &[&str],
    ) -> Element<'_, Msg> {
        let localized = |value: &str| {
            let mut item = choice(value);
            if key == "proxy_mode" && value == "system" {
                item.label = "环境代理".into();
            }
            item
        };
        let choices = options.iter().map(|s| localized(s)).collect::<Vec<_>>();
        column![
            muted(label),
            pick_list(
                choices,
                Some(localized(s(self.active_settings(), key))),
                move |v: Choice| Msg::Setting(key.into(), v.value.into())
            )
            .padding(9)
            .width(Length::Fill)
        ]
        .spacing(6)
        .into()
    }
    fn setting_check(&self, key: &'static str, label: &'static str) -> Element<'_, Msg> {
        checkbox(b(self.active_settings(), key))
            .label(label)
            .on_toggle(move |v| Msg::Setting(key.into(), v.into()))
            .size(17)
            .text_size(14)
            .into()
    }
    fn common_settings(&self) -> Element<'_, Msg> {
        let overrides = self.override_names();
        let names = overrides.as_ref().ok();
        let enabled = |name: &str| names.is_some_and(|names| names.iter().any(|item| item == name));
        let advanced_format = enabled("--format") || enabled("--preset-alias");
        let output_override = [
            "--merge-output-format",
            "--remux-video",
            "--recode-video",
            "--audio-format",
            "--extract-audio",
        ]
        .iter()
        .any(|name| enabled(name));
        let mut fields = column![
            row![
                self.setting_pick(
                    "quality",
                    if advanced_format {
                        "清晰度 · 高级格式覆盖"
                    } else {
                        "清晰度"
                    },
                    &["best", "2160", "1440", "1080", "720", "480"]
                ),
                self.setting_pick(
                    "format",
                    if output_override {
                        "格式 · 存在高级覆盖"
                    } else {
                        "格式"
                    },
                    &["auto", "mp4", "mkv", "webm", "mp3", "m4a", "flac", "wav", "opus"]
                )
            ]
            .spacing(18),
            row![
                self.setting_field("directory", "保存位置"),
                control("选择目录", Msg::Pick("directory".into()))
            ]
            .spacing(12)
            .align_y(Alignment::End),
            row![
                self.setting_check("subtitles", "字幕"),
                self.setting_check("thumbnail", "封面"),
                self.setting_check("auto_fallback", "允许降低分辨率")
            ]
            .spacing(18),
        ]
        .spacing(17);
        let mut summary = Vec::new();
        if overrides.is_err() {
            summary.push("高级参数待校验，常用设置未必生效");
        } else {
            if advanced_format {
                summary.push("由高级格式控制，上方分辨率仅为默认值");
            }
            if output_override {
                summary.push("输出格式有高级覆盖");
            }
            if enabled("--format-sort") {
                summary.push("高级画质排序");
            }
            if enabled("--output") || enabled("--paths") {
                summary.push("命名与目录覆盖");
            }
            if enabled("--limit-rate") {
                summary.push("下载限速");
            }
            if enabled("--proxy") {
                summary.push("代理覆盖");
            }
            if names.is_some_and(|names| {
                names.iter().any(|name| {
                    name.contains("cookie")
                        || name.contains("password")
                        || name.contains("username")
                })
            }) {
                summary.push("认证参数");
            }
            if names.is_some_and(|names| names.iter().any(|name| name.contains("sub"))) {
                summary.push("字幕设置");
            }
            if !s(self.active_settings(), "raw_args").trim().is_empty() {
                summary.push("已使用原生参数");
            }
        }
        if !summary.is_empty() {
            let details = names
                .map(|names| {
                    names
                        .iter()
                        .map(|name| {
                            self.catalog
                                .iter()
                                .find(|option| s(&option.data, "name") == name)
                                .map(|option| format!("{} ({name})", s(&option.data, "label")))
                                .unwrap_or_else(|| name.clone())
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "请在高级设置中检查并应用参数".into());
            fields = fields.push(tooltip(
                muted(summary.join(" · ")),
                container(text(details).size(12))
                    .padding(10)
                    .style(container::rounded_box),
                tooltip::Position::Top,
            ));
        }
        fields.into()
    }
    fn profile_pick(&self) -> Element<'_, Msg> {
        let profiles = self
            .profiles
            .iter()
            .map(|p| Choice {
                value: id(p),
                label: s(p, "name").into(),
            })
            .collect::<Vec<_>>();
        pick_list(profiles, None::<Choice>, |v: Choice| {
            Msg::ApplyProfile(v.value)
        })
        .placeholder("使用预设")
        .padding(9)
        .into()
    }
    fn tasks_view(&self) -> Element<'_, Msg> {
        let title = if self.base_page() == Page::History {
            "历史记录"
        } else {
            "下载中"
        };
        let active = self.tasks.active_count;
        let mut header = row![heading(title), Space::new().width(Length::Fill)];
        if self.base_page() == Page::Active {
            header = header.push(control(
                if self.queue_paused {
                    "继续队列"
                } else {
                    "暂停队列"
                },
                Msg::Queue,
            ));
        }
        let mut bar = row![
            self.text_field("search", "搜索标题或链接"),
            control("全选", Msg::SelectTasks(true)),
            control("取消选择", Msg::SelectTasks(false))
        ]
        .spacing(10);
        if self.base_page() == Page::History {
            bar = bar.push(
                pick_list(
                    [
                        "all",
                        "completed",
                        "failed",
                        "cancelled",
                        "interrupted",
                        "skipped",
                    ]
                    .map(choice)
                    .to_vec(),
                    Some(choice(self.field("filter"))),
                    |v: Choice| Msg::Text("filter".into(), v.value),
                )
                .padding(9),
            );
        }
        let body: Element<'_, Msg> = if self.service.is_none() {
            empty("正在启动…")
        } else if self.rows.is_empty() {
            empty(if self.base_page() == Page::History {
                "暂无历史记录"
            } else {
                "粘贴链接，开始下载"
            })
        } else {
            let (start, end, top, bottom) = self.windowed("tasks", self.rows.len(), ROW_HEIGHT);
            let mut list: Vec<(String, Element<'_, Msg>)> =
                vec![("spacer:top".into(), Space::new().height(top).into())];
            for item in &self.rows[start..end] {
                let cell: Element<'_, Msg> = match item {
                    TaskRow::Group {
                        id,
                        title,
                        count,
                        total,
                        ids,
                    } => row![
                        checkbox(ids.iter().all(|id| self.selected_tasks.contains(id)))
                            .on_toggle({
                                let ids = ids.clone();
                                move |on| Msg::GroupSelect(ids.clone(), on)
                            })
                            .size(17),
                        control(
                            if self.collapsed.contains(id) {
                                "›"
                            } else {
                                "⌄"
                            },
                            Msg::Collapse(id.clone())
                        ),
                        text(short(title, 52)).size(15).width(Length::Fill),
                        muted(format!("{count} / {total} 项"))
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into(),
                    TaskRow::Item(id) => {
                        let task = &self.tasks.tasks[id];
                        let progress = task["progress"].as_f64().unwrap_or(0.) as f32;
                        let label = if let Some(index) = task["index"].as_u64() {
                            format!("{index:02}  {}", short(s(task, "title"), 50))
                        } else {
                            short(s(task, "title"), 54)
                        };
                        let info = if s(task, "status") == "downloading" {
                            format!(
                                "{} / {} · {}/s · 剩余 {}",
                                bytes(task["downloaded"].as_f64()),
                                bytes(task["total"].as_f64()),
                                bytes(task["speed"].as_f64()),
                                duration(task["eta"].as_f64())
                            )
                        } else {
                            format!("{}  {}", status(s(task, "status")), s(task, "format_label"))
                        };
                        let content = column![
                            button(text(label).size(14))
                                .style(button::text)
                                .padding(0)
                                .on_press(Msg::Detail(id.clone())),
                            muted(info)
                        ]
                        .spacing(6)
                        .width(Length::Fill);
                        let content = if s(task, "status") == "downloading" {
                            content.push(progress_bar(0.0..=100.0, progress).girth(3).style(|_| {
                                iced::widget::progress_bar::Style {
                                    background: Color::from_rgb8(221, 222, 220).into(),
                                    bar: Color::from_rgb8(88, 123, 144).into(),
                                    border: iced::Border::default(),
                                }
                            }))
                        } else {
                            content
                        };
                        row![
                            checkbox(self.selected_tasks.contains(id))
                                .on_toggle({
                                    let id = id.clone();
                                    move |on| Msg::ToggleTask(id.clone(), on)
                                })
                                .size(17),
                            self.thumbnail(s(task, "thumbnail")),
                            content,
                            control("详情", Msg::Detail(id.clone()))
                        ]
                        .spacing(14)
                        .align_y(Alignment::Center)
                        .into()
                    }
                };
                let key = match item {
                    TaskRow::Group { id, .. } => format!("group:{id}"),
                    TaskRow::Item(id) => format!("task:{id}"),
                };
                list.push((
                    key,
                    container(cell).height(ROW_HEIGHT).padding([10, 12]).into(),
                ));
            }
            list.push(("spacer:bottom".into(), Space::new().height(bottom).into()));
            scrollable(crate::stable_rows::column(list))
                .height(Length::Fill)
                .on_scroll(|v| {
                    Msg::Scroll("tasks".into(), v.absolute_offset().y, v.bounds().height)
                })
                .into()
        };
        let mut result = column![
            header,
            muted(if self.base_page() == Page::Active {
                format!(
                    "{active} 个任务 · {}",
                    if self.queue_paused {
                        "队列已暂停，当前任务继续"
                    } else {
                        "队列运行中"
                    }
                )
            } else {
                "移除记录不会删除视频".into()
            }),
            bar,
            iced::widget::rule::horizontal(1),
            body
        ]
        .spacing(15)
        .height(Length::Fill);
        if !self.selected_tasks.is_empty() {
            result = result.push(
                column![
                    text(format!("已选 {} 项", self.selected_tasks.len())).size(13),
                    row![
                        control("取消任务", Msg::Action("cancel".into())),
                        control("原设置重试", Msg::Action("retry".into())),
                        control("当前设置重试", Msg::Action("retry_current".into())),
                        control("移除记录", Msg::Action("remove".into()))
                    ]
                    .spacing(8)
                ]
                .spacing(8),
            );
        }
        result.into()
    }
    fn windowed(&self, key: &str, count: usize, height: f32) -> (usize, usize, f32, f32) {
        let (offset, viewport) = self.offsets.get(key).copied().unwrap_or((0., 700.));
        let start = ((offset / height) as usize).saturating_sub(3).min(count);
        let end = (start + (viewport / height).ceil() as usize + 8).min(count);
        (
            start,
            end,
            start as f32 * height,
            (count - end) as f32 * height,
        )
    }
    fn add_view(&self) -> Element<'_, Msg> {
        if self.field("format_details") == "yes" {
            return self.format_view();
        }
        let mut parse = button(
            text(if self.preview_busy {
                "正在解析…"
            } else {
                "解析链接"
            })
            .size(14),
        )
        .padding([10, 18]);
        if !self.preview_busy {
            parse = parse.on_press(Msg::Preview);
        }
        let modes = pick_list(
            ["auto", "video", "playlist"].map(choice).to_vec(),
            Some(choice(self.field("mode"))),
            |v: Choice| Msg::Text("mode".into(), v.value),
        )
        .padding(10);
        let urlbar: Element<'_, Msg> = if self.compact() {
            column![
                self.text_field("url", "粘贴视频或合集链接"),
                row![modes, Space::new().width(Length::Fill), parse].spacing(10)
            ]
            .spacing(8)
            .into()
        } else {
            row![
                self.text_field("url", "粘贴视频或合集链接，多个链接用空格分隔"),
                modes,
                parse
            ]
            .spacing(10)
            .into()
        };
        let header = row![
            heading("添加下载"),
            Space::new().width(Length::Fill),
            control("关闭", Msg::ModalClose)
        ]
        .align_y(Alignment::Center);
        let visible = &self.visible_entries;
        let body: Element<'_, Msg> = if visible.is_empty() {
            empty(if self.preview_busy {
                "正在读取目录…"
            } else {
                "解析后选择要下载的视频"
            })
        } else {
            let (start, end, top, bottom) = self.windowed("entries", visible.len(), 60.);
            let mut rows: Vec<(String, Element<'_, Msg>)> =
                vec![("spacer:top".into(), Space::new().height(top).into())];
            for &index in &visible[start..end] {
                let item = &self.entries[index];
                let id = id(item);
                let label = format!(
                    "{}  {}",
                    item["index"]
                        .as_u64()
                        .map(|n| format!("{n:02}"))
                        .unwrap_or_default(),
                    short(s(item, "title"), if self.compact() { 24 } else { 50 })
                );
                let toggle = checkbox(self.selected_entries.contains(&id)).size(17);
                let toggle = if item["available"] != false && !b(item, "_partial") {
                    toggle.on_toggle(move |on| Msg::ToggleEntry(id.clone(), on))
                } else {
                    toggle
                };
                rows.push((
                    format!("entry:{}", s(item, "id")),
                    container(
                        row![
                            toggle,
                            self.thumbnail(s(item, "thumbnail")),
                            container(tooltip(
                                text(label).size(14),
                                container(text(s(item, "title")).size(13))
                                    .padding(8)
                                    .style(container::rounded_box),
                                tooltip::Position::Top
                            ))
                            .width(Length::Fill),
                            muted(duration(item["duration"].as_f64())),
                            text(if b(item, "_partial") {
                                "读取中".into()
                            } else {
                                size(item)
                            })
                            .size(12)
                            .width(100)
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .height(60)
                    .padding([8, 10])
                    .into(),
                ));
            }
            rows.push(("spacer:bottom".into(), Space::new().height(bottom).into()));
            scrollable(crate::stable_rows::column(rows))
                .height(Length::Fill)
                .on_scroll(|v| {
                    Msg::Scroll("entries".into(), v.absolute_offset().y, v.bounds().height)
                })
                .into()
        };
        let mut selectbar = row![
            control("全选", Msg::SelectVisible(true)),
            control("取消选择", Msg::SelectVisible(false)),
            text_input("1-10,15", self.field("range"))
                .on_input(|v| Msg::Text("range".into(), v))
                .padding(9)
                .width(110),
            control("选集", Msg::Range)
        ]
        .spacing(8);
        if self.preview_busy {
            selectbar = selectbar.push(control("取消解析", Msg::CancelPreview));
        }
        let mut download =
            button(text(format!("下载所选 {} 项", self.selected_entries.len())).size(15))
                .padding([12, 22]);
        if self.preview_complete && !self.selected_entries.is_empty() && !self.busy {
            download = download.on_press(Msg::Submit);
        }
        let body = container(body).height(if self.short_window() {
            Length::Fixed(180.)
        } else {
            Length::Fill
        });
        let mut left = column![
            urlbar,
            self.text_field("entry_search", "搜索合集条目"),
            selectbar,
            text(short(s(&self.preview, "title"), 80)).size(16),
            body
        ]
        .spacing(10)
        .height(if self.short_window() {
            Length::Shrink
        } else {
            Length::Fill
        });
        if self.preview_busy {
            left = left.push(muted(self.field("preview_status").to_string()));
        }
        if let Some(warnings) = self.preview["warnings"].as_array() {
            for warning in warnings {
                if let Some(warning) = warning.as_str() {
                    left = left.push(muted(warning));
                }
            }
        }
        let mut extras = row![
            control("高级设置", Msg::Page(Page::Advanced)),
            self.profile_pick(),
            control(
                if self.estimate_id.is_empty() {
                    "估算所选大小"
                } else {
                    "停止估算"
                },
                Msg::Estimate
            )
        ]
        .spacing(8);
        if self.preview["formats"]
            .as_array()
            .is_some_and(|f| !f.is_empty())
        {
            extras = extras.push(control(
                "格式详情",
                Msg::Text("format_details".into(), "yes".into()),
            ));
        }
        let settings = column![self.common_settings(), extras].spacing(12);
        let body = column![left, iced::widget::rule::horizontal(1), settings]
            .spacing(15)
            .height(if self.short_window() {
                Length::Shrink
            } else {
                Length::Fill
            });
        let body: Element<'_, Msg> = if self.short_window() {
            scrollable(body).height(Length::Fill).into()
        } else {
            body.into()
        };
        column![
            header,
            body,
            row![
                column![
                    text(&self.selection_summary).size(13),
                    muted(self.field("estimate_status").to_string())
                ]
                .width(Length::Fill),
                download
            ]
            .spacing(12)
            .align_y(Alignment::Center)
        ]
        .spacing(16)
        .height(Length::Fill)
        .into()
    }
    fn format_view(&self) -> Element<'_, Msg> {
        let mut formats = self.preview["formats"]
            .as_array()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        formats.sort_by_key(|f| std::cmp::Reverse(f["height"].as_u64().unwrap_or(0)));
        let mut list = column![
            row![
                text("格式 ID").width(70),
                text("分辨率").width(90),
                text("帧率").width(60),
                text("编码").width(Length::Fill),
                text("流大小").width(150)
            ]
            .spacing(10),
            iced::widget::rule::horizontal(1)
        ]
        .spacing(12);
        for f in formats {
            let codecs = format!("{} / {}", s(f, "vcodec"), s(f, "acodec"));
            list = list.push(
                row![
                    text(s(f, "id")).size(12).width(70),
                    text(s(f, "resolution")).size(12).width(90),
                    text(
                        f["fps"]
                            .as_f64()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "—".into())
                    )
                    .size(12)
                    .width(60),
                    text(codecs).size(12).width(Length::Fill),
                    text(format!(
                        "{}{}",
                        size(f),
                        if s(f, "size_scope") == "video_stream" {
                            "（仅视频）"
                        } else {
                            ""
                        }
                    ))
                    .size(12)
                    .width(150)
                ]
                .spacing(10),
            );
        }
        column![
            row![
                heading("格式详情"),
                Space::new().width(Length::Fill),
                control(
                    "返回下载",
                    Msg::Text("format_details".into(), String::new())
                )
            ],
            muted("视频流大小不包含单独的音频。下载总大小以所选设置的估算结果为准。"),
            scrollable(list).height(Length::Fill),
            self.setting_pick(
                "quality",
                "清晰度",
                &["best", "2160", "1440", "1080", "720", "480"]
            ),
            muted(format!(
                "当前选择：{} · 合并临时空间约 {}（粗略值）",
                s(&self.preview, "format_label"),
                bytes(self.preview["disk_estimate"].as_f64())
            ))
        ]
        .spacing(18)
        .height(Length::Fill)
        .into()
    }
    fn advanced_view(&self) -> Element<'_, Msg> {
        if !self.field("repeat_editor").is_empty() {
            return self.repeat_view();
        }
        if !self.field("option_help").is_empty() {
            return self.option_help_view();
        }
        let query = self.field("advanced_search").to_lowercase();
        let group = self.field("advanced_group");
        let mode = self.field("advanced_view");
        let settings = self.active_settings();
        let options = self
            .catalog
            .iter()
            .filter(|o| {
                let matches = query.is_empty() || o.search.contains(&query);
                matches
                    && (!query.is_empty()
                        || group.is_empty()
                        || group == "全部"
                        || if self.field("advanced_native") == "yes" {
                            o.group == group
                        } else {
                            o.section == group
                        })
                    && (!query.is_empty()
                        || mode == "all"
                        || mode == "changed"
                            && settings["advanced"].get(s(&o.data, "name")).is_some()
                        || mode == "common" && b(&o.data, "common"))
            })
            .collect::<Vec<_>>();
        let mut nav = column![text("参数").size(18), Space::new().height(8)];
        let categories = if self.field("advanced_native") == "yes" {
            let mut groups = vec!["全部".to_string()];
            for o in &self.catalog {
                if !groups.contains(&o.group) {
                    groups.push(o.group.clone());
                }
            }
            groups
        } else {
            [
                "全部",
                "画质",
                "音频",
                "字幕",
                "文件",
                "合集",
                "网络",
                "访问与认证",
                "后处理",
                "其他",
            ]
            .map(str::to_string)
            .to_vec()
        };
        for category in &categories {
            nav = nav.push(
                button(text(category.clone()).size(14))
                    .padding([9, 12])
                    .width(Length::Fill)
                    .style(if group == category {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(Msg::Text("advanced_group".into(), category.clone())),
            );
        }
        nav = nav.push(control(
            if self.field("advanced_native") == "yes" {
                "按用途分类"
            } else {
                "按原生分类"
            },
            Msg::Text(
                "advanced_native".into(),
                if self.field("advanced_native") == "yes" {
                    ""
                } else {
                    "yes"
                }
                .into(),
            ),
        ));
        let (start, end, top, bottom) = self.windowed("advanced", options.len(), 160.);
        let mut list: Vec<(String, Element<'_, Msg>)> =
            vec![("spacer:top".into(), Space::new().height(top).into())];
        for option in &options[start..end] {
            list.push((
                format!("option:{}", s(&option.data, "name")),
                self.option_view(option),
            ));
        }
        list.push(("spacer:bottom".into(), Space::new().height(bottom).into()));
        let list = crate::stable_rows::column(list);
        let controls = row![
            self.text_field("advanced_search", "搜索中文、用途或原生参数"),
            pick_list(
                ["common", "all", "changed"].map(choice).to_vec(),
                Some(choice(mode)),
                |v: Choice| Msg::Text("advanced_view".into(), v.value)
            )
            .padding(10)
        ]
        .spacing(10);
        let raw = self.field("advanced_raw") == "yes";
        let content: Element<'_, Msg> = if raw {
            column![
                text("原生参数覆盖").size(18),
                muted("支持带引号的参数文本或 JSON 数组。这里的参数优先于界面设置。"),
                text_editor(&self.raw_input)
                    .placeholder("[\"--limit-rate\", \"2M\"]")
                    .on_action(Msg::EditRaw)
                    .height(Length::Fill)
                    .padding(12)
            ]
            .spacing(12)
            .height(Length::Fill)
            .into()
        } else {
            scrollable(list)
                .height(Length::Fill)
                .on_scroll(|v| {
                    Msg::Scroll("advanced".into(), v.absolute_offset().y, v.bounds().height)
                })
                .into()
        };
        let mut main = column![controls];
        if self.compact() {
            main = main.push(
                row![
                    pick_list(categories, Some(group.to_string()), |v| Msg::Text(
                        "advanced_group".into(),
                        v
                    ))
                    .padding(8)
                    .width(Length::Fill),
                    control(
                        "分类方式",
                        Msg::Text(
                            "advanced_native".into(),
                            if self.field("advanced_native") == "yes" {
                                ""
                            } else {
                                "yes"
                            }
                            .into()
                        )
                    )
                ]
                .spacing(8),
            );
        }
        main = main
            .push(
                row![
                    muted(format!("{} / {} 项", options.len(), self.catalog.len()))
                        .width(Length::Fill),
                    self.profile_pick(),
                    control(
                        if raw { "返回参数" } else { "原生参数" },
                        Msg::Text("advanced_raw".into(), if raw { "" } else { "yes" }.into())
                    )
                ]
                .spacing(8),
            )
            .push(container(content).height(if self.short_window() {
                Length::Fixed(260.)
            } else {
                Length::Fill
            }))
            .push(
                row![
                    self.text_field("preset_name", "预设名称"),
                    control("保存预设", Msg::SaveProfile),
                    control("清空覆盖", Msg::ResetAdvanced)
                ]
                .spacing(8),
            )
            .push(
                row![
                    control("保存为默认", Msg::SaveAdvancedDefault),
                    Space::new().width(Length::Fill),
                    button("应用并返回")
                        .padding([10, 20])
                        .on_press(Msg::AdvancedBack)
                ]
                .spacing(8),
            );
        let main = main
            .spacing(12)
            .width(Length::Fill)
            .height(if self.short_window() {
                Length::Shrink
            } else {
                Length::Fill
            });
        let main: Element<'_, Msg> = if self.short_window() {
            scrollable(main).height(Length::Fill).into()
        } else {
            main.into()
        };
        if self.compact() {
            main.into()
        } else {
            row![scrollable(nav.width(150).spacing(4)).width(160), main]
                .spacing(22)
                .height(Length::Fill)
                .into()
        }
    }
    fn option_view<'a>(&'a self, option: &'a CatalogOption) -> Element<'a, Msg> {
        let data = &option.data;
        let name = s(data, "name");
        let value = &self.active_settings()["advanced"][name];
        let enabled = !value.is_null() && value != &Value::Bool(false);
        let toggle = checkbox(enabled)
            .label(s(data, "label"))
            .text_size(14)
            .size(16);
        let toggle = if b(data, "command_only") {
            toggle
        } else {
            toggle.on_toggle(move |on| Msg::AdvancedEnable(name.into(), on))
        };
        let head = row![
            container(toggle).width(Length::Fill),
            button(text(name).size(12))
                .style(button::text)
                .on_press(Msg::OptionHelp(name.into())),
            control("重置", Msg::ResetOption(name.into()))
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let widget: Element<'_, Msg> = if b(data, "command_only") {
            row![
                muted("此参数在工具中运行"),
                control("打开工具", Msg::ToolOption(name.into()))
            ]
            .spacing(12)
            .into()
        } else if !enabled || data["nargs"] == 0 {
            muted(if enabled {
                "已启用"
            } else {
                "沿用默认，勾选后设置"
            })
            .into()
        } else if b(data, "repeatable") || data["nargs"].as_u64().unwrap_or(1) > 1 {
            control("逐项编辑", Msg::Text("repeat_editor".into(), name.into())).into()
        } else if let Some(choices) = data["choices"].as_array().filter(|v| !v.is_empty()) {
            let options = choices
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            pick_list(options, value.as_str().map(str::to_owned), move |v| {
                Msg::AdvancedValue(name.into(), v)
            })
            .padding(8)
            .width(Length::Fill)
            .into()
        } else {
            let input = self.advanced_input.get(name).cloned().unwrap_or_else(|| {
                if value.is_null() {
                    String::new()
                } else if let Some(s) = value.as_str() {
                    s.to_string()
                } else {
                    value.to_string()
                }
            });
            let field = text_input(s(data, "example"), &input)
                .on_input(move |v| Msg::AdvancedValue(name.into(), v))
                .secure(b(data, "sensitive"))
                .padding(8);
            if s(data, "input_type") == "path"
                || ["path", "file", "directory"]
                    .iter()
                    .any(|term| s(data, "metavar").to_lowercase().contains(term))
            {
                row![field, control("浏览", Msg::AdvancedPick(name.into()))]
                    .spacing(8)
                    .into()
            } else {
                field.into()
            }
        };
        container(
            column![
                head,
                text(short(
                    s(data, "description"),
                    if self.compact() { 60 } else { 100 }
                ))
                .size(12),
                widget,
                muted(if value.is_null() {
                    "未设置".into()
                } else {
                    format!(
                        "已修改 · {}{}",
                        option.section,
                        if s(data, "input_type") == "number" {
                            " · 数值"
                        } else {
                            ""
                        }
                    )
                })
            ]
            .spacing(7),
        )
        .padding([10, 12])
        .height(160)
        .into()
    }
    fn option_help_view(&self) -> Element<'_, Msg> {
        let Some(option) = self
            .catalog
            .iter()
            .find(|o| s(&o.data, "name") == self.field("option_help"))
        else {
            return empty("参数不存在");
        };
        let data = &option.data;
        column![
            row![
                heading(s(data, "label")),
                Space::new().width(Length::Fill),
                control("返回高级设置", Msg::OptionHelp(String::new()))
            ],
            scrollable(
                column![
                    text(s(data, "name")).size(16),
                    text(s(data, "description")).size(14),
                    text(format!("示例：{}", s(data, "example"))).size(13),
                    text(s(data, "default_behavior")).size(13),
                    text(s(data, "help")).size(13),
                    muted(format!(
                        "别名：{}",
                        data["aliases"]
                            .as_array()
                            .map(|v| v
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("、"))
                            .unwrap_or_default()
                    ))
                ]
                .spacing(20)
            )
            .height(Length::Fill)
        ]
        .spacing(22)
        .height(Length::Fill)
        .into()
    }
    fn repeat_view(&self) -> Element<'_, Msg> {
        let name = self.field("repeat_editor");
        let option = self.catalog.iter().find(|o| s(&o.data, "name") == name);
        let title = option.map(|o| s(&o.data, "label")).unwrap_or(name);
        let mut rows = Column::new().spacing(12);
        let repeat = option.is_some_and(|o| b(&o.data, "repeatable"));
        for (index, values) in self.repeat_rows(name).into_iter().enumerate() {
            let mut fields = Column::new().spacing(8);
            for (part, value) in values.into_iter().enumerate() {
                let input: Element<'_, Msg> = if let Some(choices) = option
                    .and_then(|o| o.data["choices"].as_array())
                    .filter(|v| !v.is_empty())
                {
                    pick_list(
                        choices
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                        Some(value),
                        move |v| Msg::RepeatSet(name.into(), index, part, v),
                    )
                    .padding(10)
                    .width(Length::Fill)
                    .into()
                } else {
                    text_input(&format!("值 {}", part + 1), &value)
                        .on_input(move |v| Msg::RepeatSet(name.into(), index, part, v))
                        .secure(option.is_some_and(|o| b(&o.data, "sensitive")))
                        .padding(10)
                        .into()
                };
                fields = fields.push(input);
            }
            let mut item = row![fields.width(Length::Fill)].spacing(10);
            if repeat {
                item = item.push(control("移除", Msg::RepeatRemove(name.into(), index)));
            }
            rows = rows.push(item);
        }
        let mut view = column![
            row![
                heading(title),
                Space::new().width(Length::Fill),
                control(
                    "返回高级设置",
                    Msg::Text("repeat_editor".into(), String::new())
                )
            ],
            muted(name.to_string()),
            scrollable(rows).height(Length::Fill)
        ]
        .spacing(18)
        .height(Length::Fill);
        if repeat {
            view = view.push(control("＋ 添加一项", Msg::RepeatAdd(name.into())));
        }
        if option.is_some_and(|o| s(&o.data, "input_type") == "path" && o.data["nargs"] == 1) {
            view = view.push(control("浏览", Msg::AdvancedPick(name.into())));
        }
        view.into()
    }
    fn settings_view(&self) -> Element<'_, Msg> {
        let mut body = column![
            text("下载默认值").size(18),
            self.common_settings(),
            control("高级默认设置", Msg::Page(Page::Advanced)),
            iced::widget::rule::horizontal(1),
            text("网络与认证").size(18),
            row![
                self.setting_pick("proxy_mode", "连接方式", &["system", "direct", "custom"]),
                self.setting_field("proxy", "代理地址")
            ]
            .spacing(18),
            row![
                self.setting_field("cookie_file", "Cookies 文件"),
                control("选择文件", Msg::Pick("cookie_file".into()))
            ]
            .spacing(12)
            .align_y(Alignment::End),
            self.setting_pick(
                "cookie_browser",
                "浏览器来源",
                &["", "chrome", "edge", "firefox", "brave", "opera", "vivaldi"]
            ),
            muted("仅在你明确选择后使用认证信息；system 继承环境代理，direct 直接连接。"),
            iced::widget::rule::horizontal(1),
            text("应用").size(18),
            row![
                self.setting_pick("theme", "主题", &["system", "light", "dark"]),
                column![
                    muted("同时下载任务"),
                    pick_list(
                        vec![1, 2, 3, 4, 5, 6, 7, 8],
                        self.preferences["concurrency"].as_u64().map(|v| v as i32),
                        |v| Msg::Setting("concurrency".into(), v.into())
                    )
                    .padding(9)
                ]
            ]
            .spacing(18),
            self.setting_check(
                "close_to_tray",
                if cfg!(target_os = "macos") {
                    "关闭窗口后继续运行，可从 Dock 或菜单栏打开"
                } else {
                    "关闭窗口后保留到托盘"
                }
            ),
            self.setting_check("notifications", "任务完成和失败时通知"),
            iced::widget::rule::horizontal(1),
            text("预设").size(18),
        ]
        .spacing(18);
        for profile in &self.profiles {
            body = body.push(
                row![
                    text(s(profile, "name")).size(14).width(Length::Fill),
                    control("载入", Msg::ApplyProfile(id(profile))),
                    control("删除", Msg::DeleteProfile(id(profile)))
                ]
                .spacing(10),
            );
        }
        body = body.push(
            row![
                self.text_field("preset_name", "预设名称"),
                control("保存当前为预设", Msg::SaveProfile)
            ]
            .spacing(10),
        );
        body = body
            .push(iced::widget::rule::horizontal(1))
            .push(text("数据位置").size(18));
        if let Some(storage) = self.storage.as_object() {
            for (key, label) in [
                ("data_directory", "配置与历史"),
                ("cache_directory", "缓存"),
                ("temp_directory", "临时文件"),
            ] {
                if let Some(path) = storage.get(key).and_then(Value::as_str) {
                    body = body.push(
                        row![
                            column![muted(label), text(path).size(12)].width(Length::Fill),
                            control("打开", Msg::Open(path.into()))
                        ]
                        .spacing(10),
                    );
                }
            }
        }
        body = body.push(muted(if cfg!(target_os = "macos") {
            "视频与应用数据分开保存。移入废纸篓前，可先清理应用数据。"
        } else {
            "视频与应用数据分开保存。卸载清理应用自己的数据，不删除视频。"
        }));
        #[cfg(target_os = "macos")]
        {
            body = body.push(
                button("清理应用数据并退出")
                    .padding([8, 12])
                    .on_press_maybe(
                        self.service
                            .as_ref()
                            .filter(|service| !service.is_closed())
                            .filter(|_| !self.busy_operations.contains("clean_application_data"))
                            .map(|_| Msg::CleanApplicationData),
                    ),
            );
        }
        body = body.push(iced::widget::rule::horizontal(1)).push(
            row![
                text(format!("流光 {}", env!("CARGO_PKG_VERSION")))
                    .size(16)
                    .width(Length::Fill),
                control("开源许可", Msg::Engine("open_licenses".into()))
            ]
            .spacing(12),
        );
        column![
            row![
                heading("设置"),
                Space::new().width(Length::Fill),
                control("导入", Msg::ImportSettings),
                control("导出", Msg::ExportSettings),
                button("保存默认设置")
                    .padding([10, 18])
                    .on_press(Msg::SaveSettings)
            ]
            .spacing(10),
            scrollable(body).height(Length::Fill)
        ]
        .spacing(22)
        .height(Length::Fill)
        .into()
    }
    fn tools_view(&self) -> Element<'_, Msg> {
        let mut body = column![
            text("原生工具").size(18),
            muted(if cfg!(target_os = "macos") {
                "查询、模拟和原生参数在这里运行；二进制输出使用应用内的 flow-cli。"
            } else {
                "查询、模拟和原生参数在这里运行；二进制输出使用独立 flow.cmd。"
            }),
            row![
                control("帮助", Msg::Text("args".into(), "--help".into())),
                control("版本", Msg::Text("args".into(), "--version".into())),
                control(
                    "支持站点",
                    Msg::Text("args".into(), "--list-extractors".into())
                ),
                control(
                    "清理引擎缓存",
                    Msg::Text("args".into(), "--rm-cache-dir".into())
                )
            ]
            .spacing(8),
            text_editor(&self.tool_input)
                .placeholder("yt-dlp 参数，支持引号或 JSON 数组")
                .on_action(Msg::EditArgs)
                .height(120)
                .padding(12),
            text_editor(&self.stdin_input)
                .placeholder("标准输入（可选，文本）")
                .on_action(Msg::EditStdin)
                .height(90)
                .padding(12),
            button(if self.busy_operations.contains("run_command") {
                "正在提交…"
            } else {
                "运行"
            })
            .padding([10, 24])
            .on_press_maybe(
                (!self.busy_operations.contains("run_command")).then_some(Msg::RunCommand)
            ),
            iced::widget::rule::horizontal(1),
            text("引擎").size(18),
            muted(
                if self
                    .busy_operations
                    .iter()
                    .any(|op| op.starts_with("engine_"))
                {
                    "正在处理引擎操作…"
                } else {
                    ""
                }
            ),
            text(format!("yt-dlp {}", s(&self.engine, "version"))).size(14),
            muted(format!(
                "FFmpeg {} · FFprobe {} · JavaScript {}",
                if b(&self.engine, "ffmpeg") {
                    "可用"
                } else {
                    "缺失"
                },
                if b(&self.engine, "ffprobe") {
                    "可用"
                } else {
                    "缺失"
                },
                if b(&self.engine, "js_runtime") {
                    "可用"
                } else {
                    "缺失"
                }
            )),
            row![
                control("检查版本", Msg::Engine("engine_info".into())),
                control("更新引擎", Msg::Engine("engine_update".into())),
                control("回退", Msg::Engine("engine_rollback".into())),
                control("校验文件", Msg::Engine("engine_verify".into())),
                control("诊断", Msg::Engine("diagnostics".into()))
            ]
            .spacing(8),
        ]
        .spacing(18);
        if !self.field("tool_result").is_empty() {
            body = body.push(text(self.field("tool_result")).size(12));
        }
        column![heading("工具"), scrollable(body).height(Length::Fill)]
            .spacing(22)
            .height(Length::Fill)
            .into()
    }
    fn detail_view(&self) -> Element<'_, Msg> {
        let Some(task) = self.tasks.tasks.get(&self.detail_id) else {
            return empty("任务不存在");
        };
        let mut body = column![
            text(s(task, "title")).size(21),
            text(s(task, "url")).size(12),
            text(format!(
                "{} · {}",
                status(s(task, "status")),
                s(task, "format_label")
            ))
            .size(14),
            text(format!(
                "已下载 {} / {}",
                bytes(task["downloaded"].as_f64()),
                bytes(task["total"].as_f64())
            ))
            .size(14),
            muted(format!("引擎版本 {}", s(task, "engine_version"))),
            muted(format!(
                "创建时间 {}",
                task["created_at"]
                    .as_u64()
                    .map(flow_platform::desktop::format_timestamp)
                    .unwrap_or_else(|| "—".into())
            )),
        ]
        .spacing(14);
        if !s(task, "batch_title").is_empty() {
            body = body.push(
                text(format!(
                    "{} · 第 {} 集 / 共 {} 集",
                    s(task, "batch_title"),
                    task["index"]
                        .as_u64()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "—".into()),
                    task["batch_total"]
                        .as_u64()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "—".into())
                ))
                .size(14),
            );
        }
        if !s(task, "output").is_empty() {
            body = body.push(
                row![
                    text(s(task, "output")).size(12).width(Length::Fill),
                    control("打开所在目录", Msg::Open(s(task, "output").into()))
                ]
                .spacing(10),
            );
        }
        if let Some(media) = task["output_media"].as_object() {
            let rows = [
                ("width", "宽度"),
                ("height", "高度"),
                ("fps", "帧率"),
                ("vcodec", "视频编码"),
                ("acodec", "音频编码"),
            ];
            let mut info = Column::new().spacing(6);
            for (key, label) in rows {
                if let Some(value) = media.get(key) {
                    info = info.push(
                        row![
                            muted(label).width(90),
                            text(
                                value
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| value.to_string())
                            )
                            .size(13)
                        ]
                        .spacing(10),
                    );
                }
            }
            body = body.push(info);
        }
        if !s(task, "error").is_empty() {
            body = body.push(
                text(s(task, "error"))
                    .size(13)
                    .color(Color::from_rgb8(167, 66, 53)),
            );
        }
        let actions = if terminal(s(task, "status")) {
            row![
                control("原设置重试", Msg::DetailAction("retry".into())),
                control("当前设置重试", Msg::DetailAction("retry_current".into())),
                control("移除记录", Msg::DetailAction("remove".into()))
            ]
            .spacing(8)
        } else {
            row![control("取消任务", Msg::DetailAction("cancel".into()))]
        };
        body = body.push(actions);
        body = body.push(control(
            if self.field("detail_settings") == "yes" {
                "收起任务设置"
            } else {
                "查看任务设置与当前默认值"
            },
            Msg::Text(
                "detail_settings".into(),
                if self.field("detail_settings") == "yes" {
                    ""
                } else {
                    "yes"
                }
                .into(),
            ),
        ));
        if self.field("detail_settings") == "yes" {
            let original = exportable(&task["settings"]);
            let current = exportable(&self.settings);
            let mut comparison = column![row![
                muted("设置").width(140),
                muted("任务使用值").width(Length::Fill),
                muted("当前默认值").width(Length::Fill)
            ]
            .spacing(12)];
            if let Some(fields) = original.as_object() {
                for (key, value) in fields {
                    if matches!(
                        key.as_str(),
                        "proxy" | "cookie_file" | "cookie_browser" | "raw_args"
                    ) {
                        continue;
                    }
                    comparison = comparison.push(
                        row![
                            text(key.clone()).size(12).width(140),
                            text(
                                value
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| value.to_string())
                            )
                            .size(12)
                            .width(Length::Fill),
                            text(
                                current[key]
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| current[key].to_string())
                            )
                            .size(12)
                            .width(Length::Fill)
                        ]
                        .spacing(12),
                    );
                }
            }
            body = body.push(comparison.spacing(10)).push(muted(
                "认证信息和原生参数已隐藏。重试仍使用完整的原始设置。",
            ));
        }
        body = body.push(iced::widget::rule::horizontal(1)).push(
            row![
                text("诊断").size(18).width(Length::Fill),
                control("刷新", Msg::Detail(self.detail_id.clone())),
                control("复制", Msg::CopyLogs),
                control("导出", Msg::ExportLogs)
            ]
            .spacing(8),
        );
        let log = self.logs["lines"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        body = body.push(text(log).size(12));
        if !s(&self.logs, "output").is_empty() {
            body = body.push(control(
                if self.show_output {
                    "隐藏原始输出"
                } else {
                    "显示原始输出（可能包含敏感信息）"
                },
                Msg::RevealOutput,
            ));
            if self.show_output {
                body = body.push(text(s(&self.logs, "output")).size(12));
            }
        }
        column![
            row![
                heading("任务详情"),
                Space::new().width(Length::Fill),
                control("返回", Msg::ModalClose)
            ],
            scrollable(body).height(Length::Fill)
        ]
        .spacing(22)
        .height(Length::Fill)
        .into()
    }
}
