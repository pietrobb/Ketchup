//! A sparse symmetric matrix and its Cholesky factor, stored by rows within
//! the envelope (skyline) after a reverse Cuthill–McKee renumbering, so a
//! frame or solid mesh needs memory in proportion to its unknowns times its
//! bandwidth rather than the square of its unknowns.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// The caller's cancellation flag was set during the factorization.
    Cancelled,
}

/// The lower triangle of `P A Pᵀ`, row `i` stored from its first non-zero
/// column to the diagonal.
struct Skyline {
    /// `order[new] = old`.
    order: Vec<usize>,
    first_column: Vec<usize>,
    row_offset: Vec<usize>,
    values: Vec<f64>,
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

/// The bytes a `(row, column, value)` entry takes.
pub(crate) const ENTRY_BYTES: usize = std::mem::size_of::<(usize, usize, f64)>();

/// What a computation holds against its memory budget, taken before each
/// large allocation (R1120ac-FEA-902: the window assembled ~5.8 GB before the
/// factor's own check, and modes held several matrices beside it).
#[derive(Debug)]
pub(crate) struct MemoryBudget {
    limit: usize,
    held: usize,
}

impl MemoryBudget {
    pub(crate) const fn new(limit: usize) -> Self {
        Self { limit, held: 0 }
    }

    /// Takes `bytes` more, or says how much all would need.
    pub(crate) const fn take(&mut self, bytes: usize) -> Result<(), usize> {
        let needed = self.held.saturating_add(bytes);
        if needed > self.limit {
            return Err(needed);
        }
        self.held = needed;
        Ok(())
    }

    pub(crate) const fn give(&mut self, bytes: usize) {
        self.held = self.held.saturating_sub(bytes);
    }

    pub(crate) const fn held(&self) -> usize {
        self.held
    }

    pub(crate) const fn left(&self) -> usize {
        self.limit - self.held
    }
}

/// Sorts entries by row and column, as [`SymmetricMatrix::from_sorted`] needs.
pub(crate) fn sort_entries(entries: &mut [(usize, usize, f64)]) {
    entries.sort_unstable_by_key(|&(row, column, _)| (row, column));
}

/// The positions of the matrix that sorted `entries` fill: repeated entries
/// sum into one (R1121ac P3: a Tet4 mesh gives about six to each position, and
/// the matrix reserved, and its budget counted, a place for every one).
pub(crate) fn distinct_positions(entries: impl Iterator<Item = (usize, usize, f64)>) -> usize {
    let mut last = None;
    entries
        .filter(|&(row, column, _)| last.replace((row, column)) != Some((row, column)))
        .count()
}

impl SymmetricMatrix {
    /// Builds the matrix from `(row, column, value)` entries of both
    /// triangles; repeated entries are summed.
    pub(crate) fn from_entries(size: usize, mut entries: Vec<(usize, usize, f64)>) -> Self {
        sort_entries(&mut entries);
        Self::from_sorted(size, entries.iter().copied())
    }

    /// The bytes [`Self::from_sorted`] takes for entries filling `positions`
    /// [`distinct_positions`], at most.
    pub(crate) const fn bytes_for(size: usize, positions: usize) -> usize {
        (size + 1 + 2 * positions) * 8
    }

    /// The bytes the matrix holds.
    pub(crate) fn bytes(&self) -> usize {
        self.row_start.capacity() * std::mem::size_of::<usize>()
            + self.columns.capacity() * std::mem::size_of::<usize>()
            + self.values.capacity() * std::mem::size_of::<f64>()
    }

    /// [`Self::from_entries`] for entries already sorted by row and column.
    pub(crate) fn from_sorted(
        size: usize,
        entries: impl Iterator<Item = (usize, usize, f64)> + Clone,
    ) -> Self {
        let count = distinct_positions(entries.clone());
        let mut row_start = vec![0; size + 1];
        let mut columns = Vec::with_capacity(count);
        let mut values: Vec<f64> = Vec::with_capacity(count);
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

    /// The bytes [`Self::shifted`] takes while it builds, at most.
    pub(crate) fn shifted_bytes(&self, other: &Self) -> usize {
        let entries = self.values.len() + other.values.len();
        entries * ENTRY_BYTES + Self::bytes_for(self.size, entries)
    }

    /// `self − shift · other`, on the union of both patterns.
    pub(crate) fn shifted(&self, other: &Self, shift: f64) -> Self {
        let entries = (0..self.size)
            .flat_map(|row| {
                self.row(row)
                    .map(move |(column, value)| (row, column, value))
                    .chain(
                        other
                            .row(row)
                            .map(move |(column, value)| (row, column, -shift * value)),
                    )
            })
            .collect();
        Self::from_entries(self.size, entries)
    }

    /// How many eigenvalues of the matrix are negative: by Sylvester's law of
    /// inertia, the negative pivots `D` of its `L D Lᵀ`. `None` when a pivot
    /// is not above `pivot_threshold` in size (an eigenvalue at zero, or too
    /// near it to tell its sign). Stops between rows once `cancelled` is set.
    pub(crate) fn negative_eigenvalues(
        &self,
        pivot_threshold: f64,
        budget_bytes: usize,
        cancelled: &AtomicBool,
    ) -> Result<Option<usize>, FactorError> {
        let Skyline {
            first_column,
            row_offset,
            mut values,
            ..
        } = self.skyline(budget_bytes)?;
        let mut negative = 0;
        for row in 0..self.size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(FactorError::Cancelled);
            }
            let row_first = first_column[row];
            let (earlier, this_row) = values.split_at_mut(row_offset[row]);
            let this_row = &mut this_row[..row - row_first + 1];
            // First the products `L·D` of this row, column by column.
            for column in row_first..row {
                let start = row_first.max(first_column[column]);
                let other = &earlier[row_offset[column]..row_offset[column + 1]];
                let other_first = first_column[column];
                let known = this_row[start - row_first..column - row_first]
                    .iter()
                    .zip(&other[start - other_first..column - other_first])
                    .map(|(left, right)| left * right)
                    .sum::<f64>();
                this_row[column - row_first] -= known;
            }
            // Then `L` from them, and the pivot.
            let mut pivot = this_row[row - row_first];
            for column in row_first..row {
                let diagonal = earlier[row_offset[column + 1] - 1];
                let product = this_row[column - row_first];
                let lower = product / diagonal;
                pivot -= product * lower;
                this_row[column - row_first] = lower;
            }
            if pivot.is_nan() || pivot.abs() <= pivot_threshold {
                return Ok(None);
            }
            if pivot < 0.0 {
                negative += 1;
            }
            this_row[row - row_first] = pivot;
        }
        Ok(Some(negative))
    }

    /// The renumbered matrix's lower triangle within its envelope, unless it
    /// would need more than `budget_bytes`.
    fn skyline(&self, budget_bytes: usize) -> Result<Skyline, FactorError> {
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
        // The values with the renumbering and the rows' starts and offsets.
        let needed_bytes = entries
            .saturating_mul(std::mem::size_of::<f64>())
            .saturating_add(3 * (self.size + 1) * std::mem::size_of::<usize>());
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
        Ok(Skyline {
            order,
            first_column,
            row_offset,
            values,
        })
    }

    /// Factors the matrix as `L Lᵀ`. A pivot not above `pivot_threshold`
    /// means the matrix is singular; a factor needing more than
    /// `budget_bytes` is not attempted. Stops between rows once `cancelled`
    /// is set (og:R1119og-FEA-806: a large model factors for half a minute).
    pub(crate) fn factor(
        &self,
        pivot_threshold: f64,
        budget_bytes: usize,
        cancelled: &AtomicBool,
    ) -> Result<CholeskyFactor, FactorError> {
        let Skyline {
            order,
            first_column,
            row_offset,
            mut values,
        } = self.skyline(budget_bytes)?;
        for row in 0..self.size {
            if cancelled.load(Ordering::Relaxed) {
                return Err(FactorError::Cancelled);
            }
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
    /// The bytes the factor holds.
    pub(crate) fn bytes(&self) -> usize {
        (self.order.capacity() + self.first_column.capacity() + self.row_offset.capacity())
            * std::mem::size_of::<usize>()
            + self.values.capacity() * std::mem::size_of::<f64>()
    }

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

#[cfg(test)]
mod tests {
    use super::{FactorError, SymmetricMatrix};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn negative_pivots_count_the_eigenvalues_below_a_shift() {
        // The second difference of five unknowns: eigenvalues 2 − 2·cos(kπ/6),
        // about 0.27, 1, 2, 3 and 3.73.
        let mut entries = Vec::new();
        for row in 0..5 {
            entries.push((row, row, 2.0));
            if row > 0 {
                entries.extend([(row, row - 1, -1.0), (row - 1, row, -1.0)]);
            }
        }
        let matrix = SymmetricMatrix::from_entries(5, entries);
        let identity =
            SymmetricMatrix::from_entries(5, (0..5).map(|row| (row, row, 1.0)).collect());
        let below = |shift: f64| {
            matrix
                .shifted(&identity, shift)
                .negative_eigenvalues(1.0e-12, usize::MAX, &AtomicBool::new(false))
                .unwrap()
        };
        assert_eq!(below(0.1), Some(0));
        assert_eq!(below(1.5), Some(2));
        assert_eq!(below(3.5), Some(4));
        assert_eq!(below(4.0), Some(5));
        // A shift on an eigenvalue leaves a zero pivot: its sign is unknown.
        assert_eq!(below(2.0), None);
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            matrix
                .negative_eigenvalues(1.0e-12, usize::MAX, &cancelled)
                .err(),
            Some(FactorError::Cancelled)
        );
        assert_eq!(
            matrix.factor(1.0e-12, usize::MAX, &cancelled).err(),
            Some(FactorError::Cancelled)
        );
    }

    /// R1121ac P3: a Tet4 mesh gives each position of the matrix about six
    /// entries, one per element at the edge; the matrix holds the position once
    /// and reserves no more, and its budget counts it once.
    #[test]
    fn repeated_entries_take_one_place() {
        let entries = (0..6)
            .flat_map(|_| [(0, 0, 1.0), (0, 1, -1.0), (1, 0, -1.0), (1, 1, 1.0)])
            .collect::<Vec<_>>();
        let matrix = SymmetricMatrix::from_entries(2, entries.clone());
        let mut sorted = entries;
        super::sort_entries(&mut sorted);
        let positions = super::distinct_positions(sorted.iter().copied());
        assert_eq!(positions, 4);
        assert_eq!(matrix.bytes(), SymmetricMatrix::bytes_for(2, positions));
        assert_eq!(matrix.multiply(&[1.0, 0.0]), [6.0, -6.0]);
    }
}
