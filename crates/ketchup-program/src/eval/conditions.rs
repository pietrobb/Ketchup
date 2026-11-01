//! Library conditions report repairable design problems without aborting evaluation.
use super::*;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    fn check<'v>(
        #[starlark(require = pos)] condition: bool,
        #[starlark(require = pos)] message: &str,
        #[starlark(require = named)] parts: Value<'v>,
        #[starlark(require = named)] hint: &str,
        #[starlark(require = named, default = "error")] severity: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let severity = match severity {
            "error" => crate::validate::Severity::Error,
            "warning" => crate::validate::Severity::Warning,
            other => anyhow::bail!("check(): severity is \"error\" or \"warning\", not {other:?}"),
        };
        let heap = eval.heap();
        let parts = items(parts, heap, "check parts")?
            .into_iter()
            .map(|part| part_name(part, heap))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        for part in &parts {
            known_part(&model, part)?;
        }
        if !condition {
            model.declared_issues.push(crate::validate::Issue {
                source_lines: Vec::new(),
                severity,
                kind: crate::validate::PROGRAM_CONDITION_FAILED,
                parts,
                message: message.to_owned(),
                where_mm: None,
                hint: hint.to_owned(),
            });
        }
        Ok(condition)
    }

    /// Keeps the space of the tool body `zone` free: a part (with one of the
    /// tags `only`, when given; not one of `ignore`) reaching into it on the
    /// final model is a `free_space_occupied` error.
    fn keep_clear<'v>(
        #[starlark(require = pos)] zone: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] only: Option<Value<'v>>,
        #[starlark(require = named)] ignore: Option<Value<'v>>,
        #[starlark(require = named)] hint: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let zone = part_name(zone, heap)?;
        let ignore = given(ignore)
            .map(|value| {
                items(value, heap, "keep_clear ignore")?
                    .into_iter()
                    .map(|part| part_name(part, heap))
                    .collect::<anyhow::Result<BTreeSet<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let space = crate::clearance::FreeSpace {
            name: text(name, "keep_clear name")?.unwrap_or_else(|| zone.clone()),
            only_tags: given(only)
                .map(|value| tag_names(value, heap))
                .transpose()?
                .unwrap_or_default(),
            ignore,
            hint: text(hint, "keep_clear hint")?.unwrap_or_else(|| {
                "Move or shrink the part so it stays out of the space, or move the space's owner."
                    .to_owned()
            }),
            zone,
        };
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if model.tool(&space.zone).is_none() {
            anyhow::bail!(
                "keep_clear({:?}): the space must be a tool body (box(..., tool=True)), not a part or an unknown name",
                space.zone
            );
        }
        model.free_spaces.push(space);
        Ok(NoneType)
    }

    /// Declares load-bearing members: every part (with one of the tags `only`,
    /// when given; not one of `ignore`) must pass its weight down to the floor
    /// or an anchor. A member that is not carried is a `member_not_carried` error.
    fn load_path<'v>(
        #[starlark(require = named)] only: Option<Value<'v>>,
        #[starlark(require = named)] carriers: Option<Value<'v>>,
        #[starlark(require = named)] fixed: Option<Value<'v>>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] ignore: Option<Value<'v>>,
        #[starlark(require = named)] hint: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let ignore = given(ignore)
            .map(|value| {
                items(value, heap, "load_path ignore")?
                    .into_iter()
                    .map(|part| part_name(part, heap))
                    .collect::<anyhow::Result<BTreeSet<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let path = crate::load_path::LoadPath {
            name: text(name, "load_path name")?.unwrap_or_else(|| "load path".to_owned()),
            only_tags: given(only)
                .map(|value| tag_names(value, heap))
                .transpose()?
                .unwrap_or_default(),
            carrier_tags: given(carriers)
                .map(|value| tag_names(value, heap))
                .transpose()?
                .unwrap_or_default(),
            fixed_tags: given(fixed)
                .map(|value| tag_names(value, heap))
                .transpose()?
                .unwrap_or_default(),
            ignore,
            hint: text(hint, "load_path hint")?.unwrap_or_else(|| {
                "Rest the member from above on a carried member below it (a post, wall, \
                 plate, or a horizontal member under both of its ends), or connect it with \
                 joint(..., bearing=True) for a connector that carries it (a joist hanger, \
                 structural screws)."
                    .to_owned()
            }),
        };
        state(eval)?.model.borrow_mut().load_paths.push(path);
        Ok(NoneType)
    }

    /// How heavy a material is: kg per m³ of a part, or kg per m² of its
    /// largest face (a roofing layer modelled as one slab), with a source.
    fn material_weight<'v>(
        #[starlark(require = pos)] material: &str,
        #[starlark(require = named)] source: &str,
        #[starlark(require = named)] kg_m3: Option<Value<'v>>,
        #[starlark(require = named)] kg_m2: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let positive = |value: Option<Value<'v>>, what: &str| {
            given(value)
                .map(|value| {
                    let number = number(value, what)?;
                    if !number.is_finite() || number <= 0.0 {
                        anyhow::bail!("material_weight({material:?}): {what} must be positive");
                    }
                    Ok(number)
                })
                .transpose()
        };
        let weight = crate::loads::MaterialWeight {
            kg_m3: positive(kg_m3, "kg_m3")?,
            kg_m2: positive(kg_m2, "kg_m2")?,
            source: source.trim().to_owned(),
        };
        if weight.kg_m3.is_some() == weight.kg_m2.is_some() {
            anyhow::bail!("material_weight({material:?}): give exactly one of kg_m3 and kg_m2");
        }
        if weight.source.is_empty() {
            anyhow::bail!("material_weight({material:?}): name the source of the weight");
        }
        state(eval)?
            .model
            .borrow_mut()
            .material_weights
            .insert(material.to_owned(), weight);
        Ok(NoneType)
    }

    /// Parts with one of these tags load the members with their own weight.
    fn weight_scope<'v>(
        #[starlark(require = pos)] tags: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let tags = tag_names(tags, eval.heap())?;
        state(eval)?.model.borrow_mut().weight_scope.extend(tags);
        Ok(NoneType)
    }

    /// The parts that are made: those with one of these tags. The cut list,
    /// hardware, machining and takeoff count only them, whatever is shown;
    /// without production() every part is made.
    fn production<'v>(
        #[starlark(require = named)] tags: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let tags = tag_names(tags, eval.heap())?;
        if tags.is_empty() {
            anyhow::bail!("production(): name at least one tag");
        }
        state(eval)?
            .model
            .borrow_mut()
            .production_scope
            .extend(tags);
        Ok(NoneType)
    }

    /// A uniform load in kN/m² on the upward face of the parts tagged `on`
    /// (per plan area when `projected`). kn_m2=None declares a load whose
    /// value is not known: the members under it stay not verified.
    fn area_load<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] kind: &str,
        #[starlark(require = named)] kn_m2: Option<Value<'v>>,
        #[starlark(require = named)] on: Value<'v>,
        #[starlark(require = named, default = true)] projected: bool,
        #[starlark(require = named)] source: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        if !crate::loads::LOAD_KINDS.contains(&kind) {
            anyhow::bail!(
                "area_load({name:?}): kind must be one of {:?}, got {kind:?}",
                crate::loads::LOAD_KINDS
            );
        }
        let kn_m2 = given(kn_m2)
            .map(|value| number(value, "kn_m2"))
            .transpose()?;
        if kn_m2.is_some_and(|value| !value.is_finite() || value < 0.0) {
            anyhow::bail!("area_load({name:?}): kn_m2 must not be negative");
        }
        let load = crate::loads::AreaLoad {
            name: name.to_owned(),
            kind: kind.to_owned(),
            kn_m2,
            on: tag_names(on, heap)?,
            projected,
            source: text(source, "area_load source")?.unwrap_or_default(),
        };
        if load.kn_m2.is_some() && load.source.trim().is_empty() {
            anyhow::bail!("area_load({name:?}): name the source of the value");
        }
        state(eval)?.model.borrow_mut().area_loads.push(load);
        Ok(NoneType)
    }

    /// Strength class of a timber material for the EN 1995-1-1 member check:
    /// characteristic strengths and moduli in N/mm², the service class (1-3)
    /// and the source of the values.
    fn timber_strength<'v>(
        #[starlark(require = pos)] material: &str,
        #[starlark(require = named)] strength_class: &str,
        #[starlark(require = named)] service_class: i32,
        #[starlark(require = named)] glulam: bool,
        #[starlark(require = named)] fm_k: Value<'v>,
        #[starlark(require = named)] fv_k: Value<'v>,
        #[starlark(require = named)] fc0_k: Value<'v>,
        #[starlark(require = named)] fc90_k: Value<'v>,
        #[starlark(require = named)] e0_mean: Value<'v>,
        #[starlark(require = named)] e0_05: Value<'v>,
        #[starlark(require = named)] source: &str,
        #[starlark(require = named)] rho_k: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let positive = |value: Value<'v>, what: &str| {
            let number = number(value, what)?;
            if !number.is_finite() || number <= 0.0 {
                anyhow::bail!("timber_strength({material:?}): {what} must be positive");
            }
            Ok(number)
        };
        if !(1..=3).contains(&service_class) {
            anyhow::bail!("timber_strength({material:?}): service_class must be 1, 2 or 3");
        }
        let class = crate::member_check::TimberClass {
            strength_class: strength_class.to_owned(),
            service_class: service_class as u8,
            glulam,
            fm_k: positive(fm_k, "fm_k")?,
            fv_k: positive(fv_k, "fv_k")?,
            fc0_k: positive(fc0_k, "fc0_k")?,
            fc90_k: positive(fc90_k, "fc90_k")?,
            e0_mean: positive(e0_mean, "e0_mean")?,
            e0_05: positive(e0_05, "e0_05")?,
            rho_k: given(rho_k)
                .map(|value| positive(value, "rho_k"))
                .transpose()?,
            source: source.trim().to_owned(),
        };
        if class.source.is_empty() {
            anyhow::bail!("timber_strength({material:?}): name the source of the values");
        }
        state(eval)?
            .model
            .borrow_mut()
            .strength_classes
            .insert(material.to_owned(), class);
        Ok(NoneType)
    }

    /// The stock the made parts of `material` are cut from: bars of the
    /// stock `lengths` (each part cut to its length, one plan per cross
    /// section), or `sheets` [(length, width), ...] (one plan per thickness);
    /// `kerf` mm per saw cut, `trim` mm cut off every edge first. `along`
    /// names the part attribute whose local axis must run along a sheet's
    /// length. The report's bom.cutting lays the parts out.
    fn stock<'v>(
        #[starlark(require = pos)] material: &str,
        #[starlark(require = named)] lengths: Option<Value<'v>>,
        #[starlark(require = named)] sheets: Option<Value<'v>>,
        #[starlark(require = named)] kerf: Option<Value<'v>>,
        #[starlark(require = named)] trim: Option<Value<'v>>,
        #[starlark(require = named)] along: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let material = material.trim();
        if material.is_empty() {
            anyhow::bail!("stock(): name the material");
        }
        let measure = |value: Option<Value<'v>>, what: &str| -> anyhow::Result<f64> {
            let value = given(value).map_or(Ok(0.0), |v| number(v, what))?;
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("stock({material:?}): {what} must be zero or more mm, got {value}");
            }
            Ok(value)
        };
        let along = text(along, "stock along")?;
        let kerf_mm = measure(kerf, "kerf")?;
        let trim_mm = measure(trim, "trim")?;
        let (kind, sizes_mm) = match (given(lengths), given(sheets)) {
            (Some(lengths), None) => (
                crate::cutting::StockKind::Lengths,
                items(lengths, heap, "stock lengths")?
                    .into_iter()
                    .map(|length| Ok(vec![number(length, "stock length")?]))
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ),
            (None, Some(sheets)) => (
                crate::cutting::StockKind::Sheets,
                items(sheets, heap, "stock sheets")?
                    .into_iter()
                    .enumerate()
                    .map(|(index, sheet)| {
                        let [a, b] = numbers::<2>(sheet, heap, &format!("sheets[{index}]"))?;
                        Ok(vec![a.max(b), a.min(b)])
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ),
            _ => anyhow::bail!("stock({material:?}): give either lengths= or sheets="),
        };
        if sizes_mm.is_empty() {
            anyhow::bail!("stock({material:?}): give at least one size");
        }
        for size in &sizes_mm {
            if size
                .iter()
                .any(|side| !side.is_finite() || *side <= 2.0 * trim_mm)
            {
                anyhow::bail!(
                    "stock({material:?}): every size must be longer than twice the trim ({trim_mm} mm), got {size:?}"
                );
            }
        }
        if kind == crate::cutting::StockKind::Lengths && along.is_some() {
            anyhow::bail!("stock({material:?}): along= applies to sheets");
        }
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        // Review 2026-10-31 P3: a second stock() of a material replaced the first.
        if let Some(earlier) = model.stock.get(material) {
            anyhow::bail!(
                "stock({material:?}): stock given before ({:?} {:?}); give each material one stock",
                earlier.kind,
                earlier.sizes_mm
            );
        }
        model.stock.insert(
            material.to_owned(),
            crate::cutting::Stock {
                kind,
                sizes_mm,
                kerf_mm,
                trim_mm,
                along,
            },
        );
        Ok(NoneType)
    }
}
