use super::dependency::{Constraint, Dependency, Kind};
use super::*;

#[derive(Clone)]
struct Choice {
    item: std::sync::Arc<InstalledMod>,
    release: Option<std::sync::Arc<Release>>,
}
#[derive(Clone)]
struct State {
    selected: BTreeMap<String, Choice>,
    activation: ModList,
    assigned: HashSet<String>,
    bundled_dependencies: std::sync::Arc<BTreeMap<String, Vec<String>>>,
}
pub(super) struct Resolved {
    pub downloads: Vec<(String, Release)>,
    pub activation: ModList,
}
enum Outcome {
    Ready(Resolved),
    Metadata(String),
    Failed(Error),
}
#[derive(Default)]
struct Requirement {
    required: bool,
    constraints: Vec<Constraint>,
}
fn enabled(list: &ModList, name: &str) -> bool {
    list.mods
        .iter()
        .find(|item| item.name == name)
        .is_none_or(|item| item.enabled)
}
fn portal_choice(name: &str, release: &Release, enabled: bool) -> Choice {
    Choice {
        item: std::sync::Arc::new(InstalledMod {
            name: name.into(),
            version: release.version.clone(),
            title: name.into(),
            author: String::new(),
            description: String::new(),
            factorio_version: release.info_json.factorio_version.clone(),
            dependencies: release.info_json.dependencies.clone(),
            enabled,
            file_name: format!("{name}_{}.zip", release.version),
            size_bytes: 0,
        }),
        release: Some(std::sync::Arc::new(release.clone())),
    }
}
fn requirements(
    state: &State,
    bundled: &BTreeMap<String, String>,
    engine: &str,
) -> Result<BTreeMap<String, Requirement>> {
    let mut result: BTreeMap<String, Requirement> = BTreeMap::new();
    for choice in state.selected.values().filter(|choice| choice.item.enabled) {
        if !engine_compatible(
            &choice.item.factorio_version,
            &choice.item.dependencies,
            engine,
        ) {
            return Err(Error::bad(&format!(
                "{} is incompatible with Factorio {engine}",
                choice.item.name
            )));
        }
    }
    let sources = state
        .selected
        .values()
        .filter(|choice| choice.item.enabled)
        .map(|choice| (choice.item.name.as_str(), &choice.item.dependencies))
        .chain(
            state
                .bundled_dependencies
                .iter()
                .filter(|(name, _)| enabled(&state.activation, name))
                .map(|(name, deps)| (name.as_str(), deps)),
        );
    for (owner, dependencies) in sources {
        for text in dependencies {
            let dep = Dependency::parse(text)?;
            let present = state
                .selected
                .get(dep.name)
                .is_some_and(|choice| choice.item.enabled)
                || (bundled.contains_key(dep.name) || dep.name == "base")
                    && enabled(&state.activation, dep.name);
            if dep.kind == Kind::Incompatible {
                if present {
                    return Err(Error::bad(&format!(
                        "{} conflicts with enabled mod {}",
                        owner, dep.name
                    )));
                }
                continue;
            }
            if dep.kind == Kind::Optional && !present {
                continue;
            }
            let requirement = result.entry(dep.name.into()).or_default();
            requirement.required |= dep.kind != Kind::Optional;
            if let Some(constraint) = dep.constraint {
                requirement.constraints.push(constraint);
            }
        }
    }
    Ok(result)
}
fn solve(
    state: State,
    catalogs: &BTreeMap<String, Vec<Release>>,
    bundled: &BTreeMap<String, String>,
    engine: &str,
    budget: &mut usize,
) -> Outcome {
    if *budget == 0 || state.selected.len() > MAX_MODS || state.assigned.len() > MAX_MODS {
        return Outcome::Failed(Error::bad("Dependency resolution exceeds supported limits"));
    }
    *budget -= 1;
    let requirements = match requirements(&state, bundled, engine) {
        Ok(value) => value,
        Err(error) => return Outcome::Failed(error),
    };
    for (name, needed) in requirements {
        let satisfies = |value: &str| {
            needed
                .constraints
                .iter()
                .all(|constraint| constraint.matches(value))
        };
        if let Some(builtin) = bundled
            .get(&name)
            .map(String::as_str)
            .or_else(|| (name == "base").then_some(engine))
        {
            if !satisfies(builtin) {
                return Outcome::Failed(Error::bad(&format!(
                    "Bundled mod {name} does not satisfy required version constraints"
                )));
            }
            if !enabled(&state.activation, &name) && needed.required {
                let mut next = state.clone();
                set_entry(&mut next.activation, &name, true, builtin);
                if let Some(entry) = next
                    .activation
                    .mods
                    .iter_mut()
                    .find(|entry| entry.name == name)
                {
                    entry.version = None;
                }
                return solve(next, catalogs, bundled, engine, budget);
            }
            continue;
        }
        if state
            .selected
            .get(&name)
            .is_some_and(|choice| choice.item.enabled && satisfies(&choice.item.version))
        {
            continue;
        }
        if state.assigned.contains(&name) {
            return Outcome::Failed(Error::bad(&format!(
                "No compatible dependency set satisfies requirements for {name}"
            )));
        }
        let mut choices = Vec::new();
        if let Some(local) = state.selected.get(&name).filter(|choice| {
            satisfies(&choice.item.version)
                && engine_compatible(
                    &choice.item.factorio_version,
                    &choice.item.dependencies,
                    engine,
                )
        }) {
            let mut local = local.clone();
            std::sync::Arc::make_mut(&mut local.item).enabled = true;
            choices.push(local);
        }
        let mut last_error = Error::bad(&format!(
            "No compatible release satisfies dependency {name}"
        ));
        for local in choices {
            let mut next = state.clone();
            next.assigned.insert(name.clone());
            next.selected.insert(name.clone(), local);
            match solve(next, catalogs, bundled, engine, budget) {
                Outcome::Failed(error) => last_error = error,
                outcome => return outcome,
            }
        }
        let Some(releases) = catalogs.get(&name) else {
            return Outcome::Metadata(name);
        };
        let mut releases: Vec<_> = releases
            .iter()
            .filter(|release| {
                version(&release.version).is_ok()
                    && satisfies(&release.version)
                    && engine_compatible(
                        &release.info_json.factorio_version,
                        &release.info_json.dependencies,
                        engine,
                    )
            })
            .collect();
        releases.sort_by_key(|release| std::cmp::Reverse(version(&release.version).ok()));
        for release in releases {
            let mut next = state.clone();
            next.assigned.insert(name.clone());
            next.selected
                .insert(name.clone(), portal_choice(&name, release, true));
            match solve(next, catalogs, bundled, engine, budget) {
                Outcome::Failed(error) => last_error = error,
                outcome => return outcome,
            }
        }
        return Outcome::Failed(last_error);
    }
    let candidates: Vec<_> = state
        .selected
        .values()
        .map(|choice| choice.item.as_ref().clone())
        .collect();
    let mut activation = state.activation;
    for item in &candidates {
        set_entry(&mut activation, &item.name, item.enabled, &item.version);
    }
    if let Err(error) = check_dependencies(&candidates, &activation, Some(engine), bundled) {
        return Outcome::Failed(error);
    }
    Outcome::Ready(Resolved {
        activation,
        downloads: state
            .selected
            .into_iter()
            .filter_map(|(name, choice)| {
                choice
                    .release
                    .map(|release| (name, release.as_ref().clone()))
            })
            .collect(),
    })
}
fn initial(
    document: &ModsDocument,
    activation: ModList,
    changes: &[(String, String)],
    catalogs: &BTreeMap<String, Vec<Release>>,
    engine: &str,
) -> Result<State> {
    let mut state = State {
        selected: document
            .mods
            .iter()
            .map(|item| {
                (
                    item.name.clone(),
                    Choice {
                        item: std::sync::Arc::new(item.clone()),
                        release: None,
                    },
                )
            })
            .collect(),
        activation,
        assigned: HashSet::new(),
        bundled_dependencies: Default::default(),
    };
    for (name, target) in changes {
        if !valid_name(name) || protected(name) || !state.assigned.insert(name.clone()) {
            return Err(Error::bad("Invalid or duplicate mod selection"));
        }
        version(target)?;
        let release = catalogs
            .get(name)
            .and_then(|releases| {
                releases.iter().find(|release| {
                    release.version == *target
                        && engine_compatible(
                            &release.info_json.factorio_version,
                            &release.info_json.dependencies,
                            engine,
                        )
                })
            })
            .ok_or_else(|| Error::bad("Selected mod release is unavailable or incompatible"))?;
        let active = state
            .selected
            .get(name)
            .is_none_or(|choice| choice.item.enabled);
        state
            .selected
            .insert(name.clone(), portal_choice(name, release, active));
    }
    Ok(state)
}
fn add_catalog(
    catalogs: &mut BTreeMap<String, Vec<Release>>,
    name: String,
    releases: Vec<Release>,
    bytes: &mut usize,
) -> Result<()> {
    for release in &releases {
        *bytes = bytes
            .saturating_add(std::mem::size_of::<Release>())
            .saturating_add(release.version.len())
            .saturating_add(release.download_url.len())
            .saturating_add(release.sha1.len())
            .saturating_add(release.info_json.factorio_version.len())
            .saturating_add(release.released_at.as_ref().map_or(0, String::len));
        for dependency in &release.info_json.dependencies {
            *bytes = bytes
                .saturating_add(std::mem::size_of::<String>())
                .saturating_add(dependency.len());
        }
    }
    if *bytes > 32 * 1024 * 1024 {
        return Err(Error::bad("Dependency release metadata exceeds 32 MiB"));
    }
    catalogs.insert(name, releases);
    Ok(())
}
pub(super) async fn resolve(
    client: &reqwest::Client,
    document: &ModsDocument,
    activation: ModList,
    changes: &[(String, String)],
    inventory: &BTreeMap<String, BundledMod>,
    engine: &str,
    progress: &super::workflows::ProgressGuard,
) -> Result<Resolved> {
    let mut catalogs = BTreeMap::new();
    let mut bytes = 0;
    let mut seen = HashSet::new();
    for (name, target) in changes {
        if !valid_name(name) || protected(name) || !seen.insert(name) {
            return Err(Error::bad("Invalid or duplicate mod selection"));
        }
        version(target)?;
        add_catalog(
            &mut catalogs,
            name.clone(),
            full_releases(client, name).await?,
            &mut bytes,
        )?;
    }
    let mut state = initial(document, activation, changes, &catalogs, engine)?;
    state.bundled_dependencies = std::sync::Arc::new(
        inventory
            .iter()
            .map(|(name, item)| (name.clone(), item.dependencies.clone()))
            .collect(),
    );
    let bundled: BTreeMap<String, String> = inventory
        .iter()
        .map(|(name, item)| (name.clone(), item.version.clone()))
        .collect();
    let mut budget = 20_000;
    loop {
        let snapshot = state.clone();
        let metadata = catalogs.clone();
        let bundled_snapshot = bundled.clone();
        let engine_snapshot = engine.to_owned();
        let (outcome, remaining) = blocking(move || {
            let mut remaining = budget;
            let outcome = solve(
                snapshot,
                &metadata,
                &bundled_snapshot,
                &engine_snapshot,
                &mut remaining,
            );
            Ok((outcome, remaining))
        })
        .await?;
        budget = remaining;
        match outcome {
            Outcome::Ready(plan) => return Ok(plan),
            Outcome::Failed(error) => return Err(error),
            Outcome::Metadata(name) => {
                if catalogs.len() >= MAX_MODS || !valid_name(&name) || protected(&name) {
                    return Err(Error::bad(&format!(
                        "Cannot automatically obtain dependency {name}"
                    )));
                }
                progress.update("Resolving dependencies", 0, Some(&name))?;
                let releases = full_releases(client, &name).await?;
                add_catalog(&mut catalogs, name, releases, &mut bytes)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(value: &str, dependencies: &[&str]) -> Release {
        Release {
            version: value.into(),
            released_at: None,
            download_url: "/download/fixture/id".into(),
            sha1: "a".repeat(40),
            info_json: ReleaseInfo {
                factorio_version: "2.1".into(),
                dependencies: dependencies.iter().map(|s| (*s).into()).collect(),
            },
        }
    }
    fn resolve_fixture(
        catalogs: BTreeMap<String, Vec<Release>>,
        installed: Vec<InstalledMod>,
        list: ModList,
    ) -> Result<Resolved> {
        let doc = ModsDocument {
            revision: "fixture".into(),
            mods: installed,
            portal_authenticated: true,
        };
        let state = initial(
            &doc,
            list,
            &[("root".into(), "1.0.0".into())],
            &catalogs,
            "2.1.24",
        )?;
        match solve(state, &catalogs, &BTreeMap::new(), "2.1.24", &mut 2000) {
            Outcome::Ready(plan) => Ok(plan),
            Outcome::Failed(error) => Err(error),
            Outcome::Metadata(name) => Err(Error::bad(&format!("Fixture lacks {name}"))),
        }
    }
    fn catalog(values: &[(&str, Vec<Release>)]) -> BTreeMap<String, Vec<Release>> {
        values
            .iter()
            .map(|(name, releases)| ((*name).into(), releases.clone()))
            .collect()
    }
    #[test]
    fn gleba_colors_resolves_color_library_and_enables_bundled_space_age() -> Result<()> {
        let catalogs = catalog(&[
            (
                "functional-gleba-map-colors",
                vec![release(
                    "1.1.1",
                    &["base >= 2.0", "space-age", "hndy-color >= 1.1.1"],
                )],
            ),
            ("hndy-color", vec![release("1.1.1", &["base >= 2.0"])]),
        ]);
        let document = ModsDocument {
            revision: "fixture".into(),
            mods: vec![],
            portal_authenticated: true,
        };
        let mut activation = ModList::default();
        set_entry(&mut activation, "space-age", false, "2.1.24");
        let mut state = initial(
            &document,
            activation,
            &[("functional-gleba-map-colors".into(), "1.1.1".into())],
            &catalogs,
            "2.1.24",
        )?;
        state.bundled_dependencies = std::sync::Arc::new(BTreeMap::from([(
            "space-age".into(),
            vec!["quality".into()],
        )]));
        set_entry(&mut state.activation, "quality", false, "2.1.24");
        let bundled = BTreeMap::from([
            ("space-age".into(), "2.1.24".into()),
            ("quality".into(), "2.1.24".into()),
        ]);
        let Outcome::Ready(plan) = solve(state, &catalogs, &bundled, "2.1.24", &mut 2000) else {
            return Err(Error::bad("Expected a complete dependency plan"));
        };
        assert!(
            plan.activation
                .mods
                .iter()
                .any(|item| item.name == "quality" && item.enabled)
        );
        assert_eq!(plan.downloads.len(), 2);
        assert!(plan.downloads.iter().any(|(name, _)| name == "hndy-color"));
        assert!(
            plan.activation
                .mods
                .iter()
                .any(|item| item.name == "space-age" && item.enabled)
        );
        Ok(())
    }
    #[test]
    fn resolves_transitive_dependencies_and_ignores_absent_optional_mods() -> Result<()> {
        let plan = resolve_fixture(
            catalog(&[
                (
                    "root",
                    vec![release(
                        "1.0.0",
                        &[
                            "base >= 2.1",
                            "library >= 1.2",
                            "? absent",
                            "(?) hidden",
                            "! conflict",
                        ],
                    )],
                ),
                ("library", vec![release("1.2.0", &["~ leaf >= 2.0"])]),
                ("leaf", vec![release("2.0.0", &["base"])]),
            ]),
            Vec::new(),
            ModList::default(),
        )?;
        assert_eq!(
            plan.downloads
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["leaf", "library", "root"]
        );
        Ok(())
    }
    #[test]
    fn preserves_compatible_installed_dependencies_and_enables_disabled_ones() -> Result<()> {
        let dependency = portal_choice("library", &release("1.0.0", &["base"]), false)
            .item
            .as_ref()
            .clone();
        let plan = resolve_fixture(
            catalog(&[("root", vec![release("1.0.0", &["library >= 1.0"])])]),
            vec![dependency],
            ModList::default(),
        )?;
        assert_eq!(plan.downloads.len(), 1);
        assert!(
            plan.activation
                .mods
                .iter()
                .any(|item| item.name == "library" && item.enabled)
        );
        Ok(())
    }
    #[test]
    fn backtracks_to_a_dependency_version_with_satisfiable_transitive_requirements() -> Result<()> {
        let plan = resolve_fixture(
            catalog(&[
                (
                    "root",
                    vec![release("1.0.0", &["library >= 1.0", "leaf < 2.0"])],
                ),
                (
                    "library",
                    vec![
                        release("2.0.0", &["leaf >= 2.0"]),
                        release("1.0.0", &["leaf >= 1.0"]),
                    ],
                ),
                (
                    "leaf",
                    vec![release("2.0.0", &["base"]), release("1.0.0", &["base"])],
                ),
            ]),
            Vec::new(),
            ModList::default(),
        )?;
        assert!(
            plan.downloads
                .iter()
                .any(|(name, release)| name == "library" && release.version == "1.0.0")
        );
        Ok(())
    }
    #[test]
    fn incompatible_requirements_and_cycles_do_not_produce_a_plan() {
        for requirements in [vec!["library >= 2.0", "library < 2.0"], vec!["library"]] {
            let catalogs = catalog(&[
                ("root", vec![release("1.0.0", &requirements)]),
                ("library", vec![release("2.0.0", &["root"])]),
            ]);
            assert!(resolve_fixture(catalogs, Vec::new(), ModList::default()).is_err());
        }
    }
    #[test]
    fn cannot_disable_conflicting_existing_mods_or_change_the_requested_root_version() {
        let local = portal_choice("existing", &release("1.0.0", &["base"]), true)
            .item
            .as_ref()
            .clone();
        assert!(
            resolve_fixture(
                catalog(&[("root", vec![release("1.0.0", &["! existing"])])]),
                vec![local],
                ModList::default()
            )
            .is_err()
        );
        assert!(
            resolve_fixture(
                catalog(&[
                    ("root", vec![release("1.0.0", &["library"])]),
                    ("library", vec![release("1.0.0", &["root >= 2.0"])])
                ]),
                Vec::new(),
                ModList::default()
            )
            .is_err()
        );
    }
    #[test]
    fn enables_required_bundled_mods_without_portal_downloads() -> Result<()> {
        let catalogs = catalog(&[("root", vec![release("1.0.0", &["base"])])]);
        let mut list = ModList::default();
        list.mods[0].enabled = false;
        let plan = resolve_fixture(catalogs, Vec::new(), list)?;
        assert!(
            plan.activation
                .mods
                .iter()
                .any(|item| item.name == "base" && item.enabled)
        );
        assert_eq!(plan.downloads.len(), 1);
        Ok(())
    }
}
