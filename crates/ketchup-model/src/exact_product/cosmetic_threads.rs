//! Drawings show a modelled thread by ISO 6410, as a cosmetic one: from the
//! body its graph makes with the thread grooves left out, kept beside the
//! body that models them.
use super::*;

impl ExactResultRegistry {
    /// Keeps `cosmetic`, the body of `package`'s graph without its thread
    /// grooves, for the drawings of `package`.
    ///
    /// # Errors
    /// `package` is not held here or models no thread, or `cosmetic` is not
    /// the body of its graph without the grooves.
    pub fn insert_cosmetic_threads(
        &mut self,
        package: &ExactBodyPackage,
        cosmetic: Arc<ExactBodyPackage>,
    ) -> Result<(), ExactProductError> {
        let (ExactBodyPackage::Graph(modelled), ExactBodyPackage::Graph(plain)) =
            (package, cosmetic.as_ref())
        else {
            return Err(ExactProductError::StaleResult);
        };
        if !self.packages.contains_key(&package.result_key()) {
            return Err(ExactProductError::StaleResult);
        }
        let expected = modelled
            .graph
            .without_thread_grooves()
            .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))?
            .ok_or(ExactProductError::StaleResult)?;
        if plain.graph.graph_digest != expected.graph_digest {
            return Err(ExactProductError::StaleResult);
        }
        self.cosmetic_threads
            .insert(modelled.graph.graph_digest.clone(), cosmetic);
        self.contents_stamp = next_contents_stamp();
        Ok(())
    }

    /// The body a drawing shows for `package`: the package itself, or the
    /// body with its modelled threads cosmetic; `None` while that body has
    /// not been evaluated.
    #[must_use]
    pub fn drawing_body(&self, package: &Arc<ExactBodyPackage>) -> Option<Arc<ExactBodyPackage>> {
        match package.as_ref() {
            ExactBodyPackage::Graph(graph) if graph.graph.models_threads() => self
                .cosmetic_threads
                .get(&graph.graph.graph_digest)
                .cloned(),
            _ => Some(Arc::clone(package)),
        }
    }

    /// Keeps the cosmetic bodies `previous` has for the bodies held here.
    pub(super) fn carry_cosmetic_threads(&mut self, previous: &Self) {
        for package in self.packages.values() {
            if let ExactBodyPackage::Graph(graph) = package.as_ref()
                && let Some(cosmetic) = previous.cosmetic_threads.get(&graph.graph.graph_digest)
            {
                self.cosmetic_threads
                    .insert(graph.graph.graph_digest.clone(), Arc::clone(cosmetic));
            }
        }
    }
}
