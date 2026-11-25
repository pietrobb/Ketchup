/*
 * Copyright 2018 The Starlark in Rust Authors.
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
// Modified for Kecup (Ketchup) in 2026 from starlark 0.13.0: to_json refuses containers nested deeper than MAX_JSON_DEPTH, and repr writes them as `...`. See PATCH.md.

//! Detect recursion when doing `repr` or `to_json`.

use std::cell::Cell;

use crate::collections::SmallSet;
use crate::hint::unlikely;
use crate::values::layout::pointer::RawPointer;
use crate::values::Value;

/// Pop the stack on drop.
pub(crate) struct ReprStackGuard;

/// Pop the stack on drop.
pub(crate) struct JsonStackGuard;

impl Drop for ReprStackGuard {
    fn drop(&mut self) {
        REPR_STACK.with(|repr_stack| {
            let mut stack = Cell::take(repr_stack);
            let popped = stack.pop();
            debug_assert!(popped.is_some());
            repr_stack.set(stack);
        })
    }
}

impl Drop for JsonStackGuard {
    fn drop(&mut self) {
        JSON_STACK.with(|json_stack| {
            let mut stack = Cell::take(json_stack);
            let popped = stack.pop();
            debug_assert!(popped.is_some());
            json_stack.set(stack);
        })
    }
}

/// Why `repr` did not go into a value.
pub(crate) enum ReprRefused {
    /// The value is already being written: a cycle.
    Cycle,
    /// Ketchup patch: the value is [`MAX_JSON_DEPTH`] containers deep; written `...`.
    TooDeep,
}

/// Why `to_json` refused to go into a value.
pub(crate) enum JsonRefused {
    /// The value is already being serialized: a cycle.
    Cycle,
    /// Ketchup patch: the value is [`MAX_JSON_DEPTH`] containers deep.
    TooDeep,
}

/// Ketchup patch: how deeply containers may nest in `to_json`. Serializing
/// recurses a few kilobytes of stack per level; a list built `x = [x]` a
/// thousand times overflowed even a 64 MB stack and killed the process
/// (review 2026-11-24 R1124og-PG-905).
pub(crate) const MAX_JSON_DEPTH: usize = 200;

thread_local! {
    static REPR_STACK: Cell<SmallSet<RawPointer>> = const { Cell::new(SmallSet::new()) };
}

thread_local! {
    static JSON_STACK: Cell<SmallSet<RawPointer>> = const { Cell::new(SmallSet::new()) };
}

/// Push a value to the stack, return error if it is already on the stack.
/// Ketchup patch: or if the stack is [`MAX_JSON_DEPTH`] deep already: `str()`
/// and `repr()` of a list in a tuple 200 000 deep overflowed the stack too
/// (review 2026-11-25 R1125og-PG-1002).
pub(crate) fn repr_stack_push(value: Value) -> Result<ReprStackGuard, ReprRefused> {
    REPR_STACK.with(|repr_stack| {
        let mut stack = Cell::take(repr_stack);
        if unlikely(stack.len() >= MAX_JSON_DEPTH) {
            repr_stack.set(stack);
            Err(ReprRefused::TooDeep)
        } else if unlikely(!stack.insert(value.ptr_value())) {
            repr_stack.set(stack);
            Err(ReprRefused::Cycle)
        } else {
            repr_stack.set(stack);
            Ok(ReprStackGuard)
        }
    })
}

/// Push a value to the stack, return error if it is already on the stack.
/// Ketchup patch: or if the stack is [`MAX_JSON_DEPTH`] deep already.
pub(crate) fn json_stack_push(value: Value) -> Result<JsonStackGuard, JsonRefused> {
    JSON_STACK.with(|json_stack| {
        let mut stack = Cell::take(json_stack);
        if unlikely(stack.len() >= MAX_JSON_DEPTH) {
            json_stack.set(stack);
            Err(JsonRefused::TooDeep)
        } else if unlikely(!stack.insert(value.ptr_value())) {
            json_stack.set(stack);
            Err(JsonRefused::Cycle)
        } else {
            json_stack.set(stack);
            Ok(JsonStackGuard)
        }
    })
}
