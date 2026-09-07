//! 读取侧计数器：回答「同一个元素被重复取了多少次」。
//!
//! [`DbElement`](crate::db_element::DbElement) 的 `parsed` / `extraction` 是**逐句柄**
//! 记忆化的（各一个 `OnceLock`），而 [`DbSet::element`](crate::db_element::DbSet::element)
//! 每次都造一个新句柄。于是同一个 refno 被两条路径拿到时，索引查找 + 记录读 + 解析
//! 会整套重做一遍——典型的重复源是 owner 链上溯（每个元素都把祖先重走一遍）与
//! 目录链走查（同一只目录件被成百上千个成员引用）。
//!
//! 这一层只统计，不改行为：命中一次加一个计数，未命中额外记耗时。判断「要不要给
//! `DbSet` 加共享解析缓存」靠的就是 miss/hit 比和 miss 耗时占比，不是感觉。
//!
//! 计数是进程全局的 relaxed 原子量。命中路径只有一个 `fetch_add`，相对一次 B-tree
//! 查找可以忽略；但多线程下它们共享缓存行，做并行基准时先 [`reset`] 再取
//! [`snapshot`]，别把计数器自身的争用读成生成的开销。
//!
//! 除了计数，这里还有一件**按需开**的记录器：[`trace_element_reads`] 在当前线程上开一段
//! 作用域，作用域内每一个真被读到的元素（`parsed` / `extraction` 走到并读成功，不只是造了句柄）
//! 按首次读到的顺序记下 refno。它回答的不是「读了多少次」而是「读了**哪些**」——
//! e3d-model 用它给一次生成交出 `catalogue_visited`（gen-model ADR-064 D5 的对拍 oracle）。
//! 记录器是线程局部的：生成是同步 DFS，跨线程的读不属于这一次生成。没开作用域时每次读
//! 只多一次线程局部计数器的读取。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::refno::RefNo;

static PARSE_HITS: AtomicU64 = AtomicU64::new(0);
static PARSE_SHARED_HITS: AtomicU64 = AtomicU64::new(0);
static PARSE_MISSES: AtomicU64 = AtomicU64::new(0);
static PARSE_NANOS: AtomicU64 = AtomicU64::new(0);
static EXTRACT_HITS: AtomicU64 = AtomicU64::new(0);
static EXTRACT_SHARED_HITS: AtomicU64 = AtomicU64::new(0);
static EXTRACT_MISSES: AtomicU64 = AtomicU64::new(0);
static EXTRACT_NANOS: AtomicU64 = AtomicU64::new(0);
static ATTR_LOOKUPS: AtomicU64 = AtomicU64::new(0);

/// 某一刻的读取计数。差值用 [`ReadStats::delta_from`]，别自己相减——
/// 少减一项就得到一个看着像真的数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// 句柄自己的 `OnceLock` 命中次数（零 I/O）。
    pub parse_hits: u64,
    /// 池级缓存命中次数（[`DbSet::with_element_cache`](crate::db_element::DbSet::with_element_cache)
    /// 打开时才可能非零）。它与 `parse_hits` 分开记：这一档正是「跨句柄的重复」，
    /// 合进去就看不出开关到底省了什么。
    pub parse_shared_hits: u64,
    /// 真去查索引 + 读记录 + 解析的次数。
    pub parse_misses: u64,
    /// 未命中路径的累计耗时（纳秒）。
    pub parse_nanos: u64,
    pub extract_hits: u64,
    pub extract_shared_hits: u64,
    pub extract_misses: u64,
    pub extract_nanos: u64,
    /// 按名字取属性的次数（[`DbElement::attribute`](crate::db_element::DbElement::attribute)
    /// 一族）。取到抽取件之后还要在名字表里找那一条，这一档量的是后半程。
    ///
    /// 2026-09-04 实测 ams8000 一次生成 104 万次，当时是顺序扫、平均比 9 个名字，
    /// 于是有了
    /// [`AttrNameIndex`](crate::record::descriptor::AttrNameIndex)。留着这个计数
    /// 是为了下次有人怀疑那张索引时，先看这个数还在不在同一个量级。
    pub attr_lookups: u64,
}

impl ReadStats {
    pub fn parse_time(&self) -> Duration {
        Duration::from_nanos(self.parse_nanos)
    }

    pub fn extract_time(&self) -> Duration {
        Duration::from_nanos(self.extract_nanos)
    }

    /// 本区间的增量。全局计数器是进程级的，跨库或跨阶段比较一律走这个。
    pub fn delta_from(&self, earlier: ReadStats) -> ReadStats {
        ReadStats {
            parse_hits: self.parse_hits.saturating_sub(earlier.parse_hits),
            parse_shared_hits: self
                .parse_shared_hits
                .saturating_sub(earlier.parse_shared_hits),
            parse_misses: self.parse_misses.saturating_sub(earlier.parse_misses),
            parse_nanos: self.parse_nanos.saturating_sub(earlier.parse_nanos),
            extract_hits: self.extract_hits.saturating_sub(earlier.extract_hits),
            extract_shared_hits: self
                .extract_shared_hits
                .saturating_sub(earlier.extract_shared_hits),
            extract_misses: self.extract_misses.saturating_sub(earlier.extract_misses),
            extract_nanos: self.extract_nanos.saturating_sub(earlier.extract_nanos),
            attr_lookups: self.attr_lookups.saturating_sub(earlier.attr_lookups),
        }
    }

    /// 一行摘要，直接进日志。
    pub fn summary(&self) -> String {
        format!(
            "parse hit={} shared={} miss={} {}ms | extract hit={} shared={} miss={} {}ms | attr lookups={}",
            self.parse_hits,
            self.parse_shared_hits,
            self.parse_misses,
            self.parse_time().as_millis(),
            self.extract_hits,
            self.extract_shared_hits,
            self.extract_misses,
            self.extract_time().as_millis(),
            self.attr_lookups,
        )
    }
}

pub fn snapshot() -> ReadStats {
    ReadStats {
        parse_hits: PARSE_HITS.load(Ordering::Relaxed),
        parse_shared_hits: PARSE_SHARED_HITS.load(Ordering::Relaxed),
        parse_misses: PARSE_MISSES.load(Ordering::Relaxed),
        parse_nanos: PARSE_NANOS.load(Ordering::Relaxed),
        extract_hits: EXTRACT_HITS.load(Ordering::Relaxed),
        extract_shared_hits: EXTRACT_SHARED_HITS.load(Ordering::Relaxed),
        extract_misses: EXTRACT_MISSES.load(Ordering::Relaxed),
        extract_nanos: EXTRACT_NANOS.load(Ordering::Relaxed),
        attr_lookups: ATTR_LOOKUPS.load(Ordering::Relaxed),
    }
}

/// 清零。基准跑之间用；生产路径不调。
pub fn reset() {
    for counter in [
        &PARSE_HITS,
        &PARSE_SHARED_HITS,
        &PARSE_MISSES,
        &PARSE_NANOS,
        &EXTRACT_HITS,
        &EXTRACT_SHARED_HITS,
        &EXTRACT_MISSES,
        &EXTRACT_NANOS,
        &ATTR_LOOKUPS,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

pub(crate) fn note_parse_hit() {
    PARSE_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_parse_shared_hit() {
    PARSE_SHARED_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_extract_shared_hit() {
    EXTRACT_SHARED_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_parse_miss(elapsed: Duration) {
    PARSE_MISSES.fetch_add(1, Ordering::Relaxed);
    PARSE_NANOS.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
}

pub(crate) fn note_extract_hit() {
    EXTRACT_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_extract_miss(elapsed: Duration) {
    EXTRACT_MISSES.fetch_add(1, Ordering::Relaxed);
    EXTRACT_NANOS.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
}

/// 一次按名字取属性。
pub(crate) fn note_attr_lookup() {
    ATTR_LOOKUPS.fetch_add(1, Ordering::Relaxed);
}

// ---- 按需开的「读了哪些元素」记录器（线程局部、可嵌套） ---------------------

/// 一层作用域：去重集 + 首次读到的顺序。
#[derive(Default)]
struct TraceFrame {
    seen: HashSet<RefNo>,
    order: Vec<RefNo>,
}

thread_local! {
    /// 打开着的作用域数。读路径先看它，为零就一个字都不记——记录器关着时的全部开销。
    static TRACE_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// 作用域栈，最内层在末尾。嵌套时一次读记进**每一层**：外层要的是它整段里读过的全部元素，
    /// 内层开过一段不该把这些读从外层的账上拿走。
    static TRACE_FRAMES: RefCell<Vec<TraceFrame>> = const { RefCell::new(Vec::new()) };
}

/// 一段正在记录的作用域。用 [`ReadTrace::finish`] 收账；直接 drop 则丢掉这一层的记录
/// （`?` 早退的路径不会留下一层挂着的作用域）。
///
/// 作用域**按后开先收**配对——它们是一个栈；乱序收账在 debug 构建里断言，release 下
/// 收的是最内层那一层。
#[must_use = "不 finish 就拿不到读到的元素；只想开关请别调这个函数"]
pub struct ReadTrace {
    /// 开这一层时的栈深（1 起），收账时核对配对。
    depth: usize,
}

/// 在当前线程开一段「记录读到的元素」作用域。
///
/// 记的是**读成功**的元素：[`DbElement::parsed`](crate::db_element::DbElement::parsed) 或
/// [`DbElement::extraction`](crate::db_element::DbElement::extraction) 走到并回了 `Ok`——包括句柄级
/// 与池级缓存命中的那几次（缓存命中仍是一次读，只是没花 I/O）；只造了句柄
/// （[`DbSet::element`](crate::db_element::DbSet::element) / `get_element`）没读它的不算，
/// 索引里没有的 refno（悬空引用，`NotFound`）也不算——它不是一个元素。
pub fn trace_element_reads() -> ReadTrace {
    let depth = TRACE_FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        frames.push(TraceFrame::default());
        frames.len()
    });
    TRACE_DEPTH.with(|d| d.set(depth));
    ReadTrace { depth }
}

impl ReadTrace {
    /// 收账：这一层作用域内读到的元素，按首次读到的顺序去重。
    pub fn finish(self) -> Vec<RefNo> {
        let frame = TRACE_FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            debug_assert_eq!(
                frames.len(),
                self.depth,
                "ReadTrace 收账顺序乱了：作用域是一个栈，后开的先收"
            );
            frames.pop()
        });
        TRACE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        // 栈已经弹过了，drop 不能再弹一次。
        std::mem::forget(self);
        frame.map(|frame| frame.order).unwrap_or_default()
    }
}

impl Drop for ReadTrace {
    fn drop(&mut self) {
        TRACE_FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            debug_assert_eq!(
                frames.len(),
                self.depth,
                "ReadTrace 丢弃顺序乱了：作用域是一个栈，后开的先丢"
            );
            frames.pop();
        });
        TRACE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// 一个元素真被读到了（`parsed` / `extraction` 的入口）。没开作用域时只读一次线程局部计数器。
pub(crate) fn note_element_read(refno: RefNo) {
    if TRACE_DEPTH.with(|d| d.get()) == 0 {
        return;
    }
    TRACE_FRAMES.with(|frames| {
        for frame in frames.borrow_mut().iter_mut() {
            if frame.seen.insert(refno) {
                frame.order.push(refno);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 差值只认区间内的增量：跨阶段比较全靠它，自己相减少一项就得到假数。
    #[test]
    fn delta_reports_only_the_interval() {
        let earlier = ReadStats {
            parse_hits: 10,
            parse_shared_hits: 2,
            parse_misses: 3,
            parse_nanos: 500,
            extract_hits: 1,
            extract_shared_hits: 0,
            extract_misses: 1,
            extract_nanos: 100,
            attr_lookups: 4,
        };
        let later = ReadStats {
            parse_hits: 25,
            parse_shared_hits: 9,
            parse_misses: 8,
            parse_nanos: 1500,
            extract_hits: 4,
            extract_shared_hits: 6,
            extract_misses: 2,
            extract_nanos: 350,
            attr_lookups: 30,
        };
        let delta = later.delta_from(earlier);
        assert_eq!(delta.parse_hits, 15);
        assert_eq!(delta.parse_shared_hits, 7);
        assert_eq!(delta.extract_shared_hits, 6);
        assert_eq!(delta.parse_misses, 5);
        assert_eq!(delta.parse_nanos, 1000);
        assert_eq!(delta.extract_hits, 3);
        assert_eq!(delta.extract_misses, 1);
        assert_eq!(delta.extract_nanos, 250);
        assert_eq!(delta.attr_lookups, 26);
    }

    fn refno(dbno: u32, word1: u32) -> RefNo {
        RefNo::new(dbno, word1)
    }

    /// 作用域内的读按首次顺序去重；作用域外的读一个都不记；嵌套时内层的读同时记进外层。
    #[test]
    fn a_trace_records_each_read_once_in_first_seen_order_and_nests() {
        note_element_read(refno(7000, 1)); // 没开作用域：不记
        let outer = trace_element_reads();
        note_element_read(refno(8000, 10));
        note_element_read(refno(7000, 2));
        note_element_read(refno(8000, 10)); // 重复读只记一次
        let inner = trace_element_reads();
        note_element_read(refno(7000, 3));
        note_element_read(refno(7000, 2)); // 外层已见、内层首次
        assert_eq!(inner.finish(), vec![refno(7000, 3), refno(7000, 2)]);
        note_element_read(refno(7000, 4));
        assert_eq!(
            outer.finish(),
            vec![refno(8000, 10), refno(7000, 2), refno(7000, 3), refno(7000, 4)]
        );
        // 全部收完之后又回到「不记」。
        note_element_read(refno(7000, 5));
        assert_eq!(TRACE_DEPTH.with(|d| d.get()), 0);
        assert!(TRACE_FRAMES.with(|frames| frames.borrow().is_empty()));
    }

    /// `?` 早退把作用域直接 drop 掉：那一层的记录丢弃、栈弹回，外层不受影响。
    #[test]
    fn dropping_a_trace_unwinds_its_frame_without_touching_the_outer_one() {
        let outer = trace_element_reads();
        {
            let _inner = trace_element_reads();
            note_element_read(refno(7000, 9));
        }
        assert_eq!(TRACE_DEPTH.with(|d| d.get()), 1);
        note_element_read(refno(7000, 8));
        assert_eq!(outer.finish(), vec![refno(7000, 9), refno(7000, 8)]);
        assert_eq!(TRACE_DEPTH.with(|d| d.get()), 0);
    }

    /// 计数器是进程全局的，别的用例先跑过就会让差值为负——饱和减是刻意的。
    #[test]
    fn a_reversed_interval_saturates_instead_of_wrapping() {
        let later = ReadStats {
            parse_hits: 1,
            ..Default::default()
        };
        assert_eq!(
            later
                .delta_from(ReadStats {
                    parse_hits: 9,
                    ..Default::default()
                })
                .parse_hits,
            0
        );
    }
}
