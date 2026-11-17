//! A sparse symmetric matrix and its Cholesky factor, stored by rows within
//! the envelope (skyline) after a reverse Cuthill–McKee renumbering, so a
//! frame or solid mesh needs memory in proportion to its unknowns times its
//! bandwidth rather than the square of its unknowns.

use std::collections::VecDeque;

/// A symmetric matrix assembled entry by entry. Both triangles are kept so
/// a row can be read whole.
#[derive(Clone, Debug)]
pub(crate) struct SymmetricMatrix {
    size: usize,
    row_start: Vec<usize>,
    columns: Vec<usize>,
    values: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FactorError {
    /// The factor would need `needed_bytes`, more than the budget.
    TooLarge { needed_bytes: usize },
    /// A pivot at or below the threshold: the matrix is singular or not
    /// positive definite.
    Singular,
}

/// The lower Cholesky factor `L` of `P A Pᵀ`, row `i` stored from its first
/// non-zero column to the diagonal.
pub(crate) struct CholeskyFactor {
    /// `order[new] = old`.
    order: Vec<usize>,
    first_column: Vec<usize>,
    row_offset: Vec<usize>,
    values: Vec<f64>,
}

impl SymmetricMatrix {
    /// Builds the matrix from `(row, column, value)` entries of both
    /// triangles; repeated entries are summed.
    pub(crate) fn from_entries(size: usize, mut entries: Vec<(usize, usize, f64)>) -> Self {
        entries.sort_unstable_by_key(|&(row, column, _)| (row, column));
        let mut row_start = vec![0; size + 1];
        let mut columns = Vec::with_capacity(entries.len());
        let mut values: Vec<f64> = Vec::with_capacity(entries.len());
        let mut last = None;
        for (row, column, value) in entries {
            if last == Some((row, column)) {
                if let Some(sum) = values.last_mut() {
                    *sum += value;
                }
                continue;
            }
            last = Some((row, column));
            columns.push(column);
            values.push(value);
            row_start[row + 1] = columns.len();
        }
        for row in 0..size {
            row_start[row + 1] = row_start[row + 1].max(row_start[row]);
        }
        Self {
            size,
            row_start,
            columns,
            values,
        }
    }

    fn row(&self, row: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let range = self.row_start[row]..self.row_start[row + 1];
        self.columns[range.clone()]
            .iter()
            .copied()
            .zip(self.values[range].iter().copied())
    }

    pub(crate) fn largest_magnitude(&self) -> f64 {
        self.values
            .iter()
            .map(|value| value.abs())
            .fold(0.0, f64::max)
    }

    pub(crate) fn multiply(&self, vector: &[f64]) -> Vec<f64> {
        (0..self.size)
            .map(|row| {
                self.row(row)
                    .map(|(column, value)| value * vector[column])
                    .sum()
            })
            .collect()
    }

    /// Fixes unknown `dof` at `value` symmetrically: its row and column are
    /// cleared, the known part moves to the right-hand side and the diagonal
    /// becomes `scale`.
    pub(crate) fn prescribe(&mut self, dof: usize, value: f64, scale: f64, rhs: &mut [f64]) {
        let range = self.row_start[dof]..self.row_start[dof + 1];
        for index in range {
            let column = self.columns[index];
            if column == dof {
                self.values[index] = scale;
                continue;
            }
            // Column `dof` of row `column` holds the same value; the pattern
            // is built symmetric, so it is there.
            if let Some(mirror) = self.position_of(column, dof) {
                rhs[column] -= self.values[mirror] * value;
                self.values[mirror] = 0.0;
            }
            self.values[index] = 0.0;
        }
        rhs[dof] = scale * value;
    }

    fn position_of(&self, row: usize, column: usize) -> Option<usize> {
        let range = self.row_start[row]..self.row_start[row + 1];
        self.columns[range.clone()]
            .binary_search(&column)
            .ok()
            .map(|offset| range.start + offset)
    }

    /// A reverse Cuthill–McKee order of the unknowns: each connected part
    /// starts from a far end of it and takes neighbours by rising degree.
    fn bandwidth_order(&self) -> Vec<usize> {
        let degree = |row: usize| self.row_start[row + 1] - self.row_start[row];
        let mut placed = vec![false; self.size];
        let mut order = Vec::with_capacity(self.size);
        let mut by_degree = (0..self.size).collect::<Vec<_>>();
        by_degree.sort_by_key(|&row| degree(row));
        for start in by_degree {
            if placed[start] {
                continue;
            }
            let start = self.far_end(start, &placed);
            placed[start] = true;
            let mut queue = VecDeque::from([start]);
            while let Some(row) = queue.pop_front() {
                order.push(row);
                let mut next = self
                    .row(row)
                    .map(|(column, _)| column)
                    .filter(|&column| !placed[column])
                    .collect::<Vec<_>>();
                next.sort_by_key(|&column| (degree(column), column));
                for column in next {
                    placed[column] = true;
                    queue.push_back(column);
                }
            }
        }
        order.reverse();
        order
    }

    /// The last unknown reached by breadth-first searches from `start`,
    /// repeated while the search grows deeper (a pseudo-peripheral unknown).
    fn far_end(&self, start: usize, placed: &[bool]) -> usize {
        let mut current = start;
        let mut depth = 0;
        loop {
            let (last, reached) = self.deepest(current, placed);
            if reached <= depth {
                return current;
            }
            depth = reached;
            current = last;
        }
    }

    fn deepest(&self, start: usize, placed: &[bool]) -> (usize, usize) {
        let mut level = vec![usize::MAX; self.size];
        level[start] = 0;
        let mut queue = VecDeque::from([start]);
        let mut last = (start, 0);
        while let Some(row) = queue.pop_front() {
            // Breadth first, so the last unknown taken is one of the deepest.
            last = (row, level[row]);
            for (column, _) in self.row(row) {
                if !placed[column] && level[column] == usize::MAX {
                    level[column] = level[row] + 1;
                    queue.push_back(column);
                }
            }
        }
        last
    }

    /// Factors the matrix as `L Lᵀ`. A pivot not above `pivot_threshold`
    /// means the matrix is singular; a factor needing more than
    /// `budget_bytes` is not attempted.
    pub(crate) fn factor(
        &self,
        pivot_threshold: f64,
        budget_bytes: usize,
    ) -> Result<CholeskyFactor, FactorError> {
        let order = self.bandwidth_order();
        let mut position = vec![0; self.size];
        for (new, &old) in order.iter().enumerate() {
            position[old] = new;
        }
        let mut first_column = (0..self.size).collect::<Vec<_>>();
        for (new, &old) in order.iter().enumerate() {
            for (column, _) in self.row(old) {
                let column = position[column];
                if column < first_column[new] {
                    first_column[new] = column;
                }
            }
        }
        let mut row_offset = Vec::with_capacity(self.size + 1);
        let mut entries: usize = 0;
        for (row, &first) in first_column.iter().enumerate() {
            row_offset.push(entries);
            entries = entries.saturating_add(row - first + 1);
        }
        row_offset.push(entries);
        let needed_bytes = entries.saturating_mul(std::mem::size_of::<f64>());
        if needed_bytes > budget_bytes {
            return Err(FactorError::TooLarge { needed_bytes });
        }

        let mut values = vec![0.0; entries];
        for (new, &old) in order.iter().enumerate() {
            for (column, value) in self.row(old) {
                let column = position[column];
                if column <= new {
                    values[row_offset[new] + column - first_column[new]] += value;
                }
            }
        }
        for row in 0..self.size {
            let row_first = first_column[row];
            for column in row_first..=row {
                let start = row_first.max(first_column[column]);
                let (earlier, this_row) = values.split_at_mut(row_offset[row]);
                let this_row = &mut this_row[..row - row_first + 1];
                let known = if column == row {
                    this_row[start - row_first..column - row_first]
                        .iter()
                        .map(|value| value * value)
                        .sum::<f64>()
                } else {
                    let other = &earlier[row_offset[column]..row_offset[column + 1]];
                    let other_first = first_column[column];
                    this_row[start - row_first..column - row_first]
                        .iter()
                        .zip(&other[start - other_first..column - other_first])
                        .map(|(left, right)| left * right)
                        .sum::<f64>()
                };
                let entry = &mut this_row[column - row_first];
                if column == row {
                    let pivot = *entry - known;
                    if pivot.is_nan() || pivot <= pivot_threshold {
                        return Err(FactorError::Singular);
                    }
                    *entry = pivot.sqrt();
                } else {
                    let diagonal = earlier[row_offset[column + 1] - 1];
                    *entry = (*entry - known) / diagonal;
                }
            }
        }
        Ok(CholeskyFactor {
            order,
            first_column,
            row_offset,
            values,
        })
    }
}

impl CholeskyFactor {
    pub(crate) fn solve(&self, rhs: &[f64]) -> Vec<f64> {
        let size = self.order.len();
        let mut work = self.order.iter().map(|&old| rhs[old]).collect::<Vec<_>>();
        for row in 0..size {
            let stored = &self.values[self.row_offset[row]..self.row_offset[row + 1]];
            let first = self.first_column[row];
            let known = stored[..row - first]
                .iter()
                .zip(&work[first..row])
                .map(|(left, right)| left * right)
                .sum::<f64>();
            work[row] = (work[row] - known) / stored[row - first];
        }
        for row in (0..size).rev() {
            let stored = &self.values[self.row_offset[row]..self.row_offset[row + 1]];
            let first = self.first_column[row];
            work[row] /= stored[row - first];
            let value = work[row];
            for (entry, target) in stored[..row - first].iter().zip(&mut work[first..row]) {
                *target -= entry * value;
            }
        }
        let mut solution = vec![0.0; size];
        for (new, &old) in self.order.iter().enumerate() {
            solution[old] = work[new];
        }
        solution
    }
}
