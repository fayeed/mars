mod support;

use mars_brain::{BrainError, BrainService};

use support::indexed_fixture;

#[test]
fn structural_tools_return_symbols_and_evidence_backed_edges() {
    let fixture = indexed_fixture();
    let brain = BrainService::new(fixture.storage);

    let add = brain
        .find_symbol(&fixture.repository_id, "add", 10)
        .expect("symbol search should work");
    assert!(!add.is_empty());
    assert_eq!(add[0].name, "add");
    assert_eq!(add[0].relative_path, "src/math.ts");
    assert!(add[0].start_line > 0);

    let shared = brain
        .find_symbol(&fixture.repository_id, "sharedValue", 10)
        .expect("shared symbol should load");
    let references = brain
        .references(&fixture.repository_id, &shared[0].id, 20)
        .expect("references should load");
    assert!(references.iter().any(|edge| {
        edge.source
            .as_ref()
            .is_some_and(|source| source.name == "run")
            && edge.evidence.relative_path == "src/main.ts"
    }));

    let calculate = brain
        .find_symbol(&fixture.repository_id, "calculate", 10)
        .expect("method should load");
    let callers = brain
        .callers(&fixture.repository_id, &calculate[0].id, 20)
        .expect("callers should load");
    assert!(callers.iter().any(|edge| {
        edge.source
            .as_ref()
            .is_some_and(|source| source.name == "run")
    }));

    let run = brain
        .find_symbol(&fixture.repository_id, "run", 10)
        .expect("run should load");
    let callees = brain
        .callees(&fixture.repository_id, &run[0].id, 20)
        .expect("callees should load");
    assert!(callees.iter().any(|edge| {
        edge.target
            .as_ref()
            .is_some_and(|target| target.name == "calculate")
    }));
}

#[test]
fn dependencies_and_bounded_neighborhoods_are_explainable() {
    let fixture = indexed_fixture();
    let brain = BrainService::new(fixture.storage);
    let dependencies = brain
        .dependencies(&fixture.repository_id, 50)
        .expect("dependencies should load");
    assert!(dependencies.iter().any(|dependency| {
        dependency.relative_path == "src/main.ts" && dependency.specifier == "./math"
    }));

    let add = brain
        .find_symbol(&fixture.repository_id, "add", 10)
        .expect("add should load");
    let related = brain
        .related_code(&fixture.repository_id, &add[0].id, 2, 25)
        .expect("related code should load");
    assert!(
        related
            .symbols
            .iter()
            .any(|symbol| symbol.name == "calculate")
    );
    assert!(
        related
            .edges
            .iter()
            .all(|edge| !edge.evidence.relative_path.is_empty())
    );

    let blast = brain
        .blast_radius(&fixture.repository_id, &add[0].id, 2, 25)
        .expect("blast radius should load");
    assert!(
        blast
            .symbols
            .iter()
            .any(|symbol| symbol.name == "calculate")
    );

    assert!(matches!(
        brain.related_code(&fixture.repository_id, &add[0].id, 0, 25),
        Err(BrainError::InvalidBounds { .. })
    ));
    assert!(matches!(
        brain.blast_radius(&fixture.repository_id, &add[0].id, 6, 25),
        Err(BrainError::InvalidBounds { .. })
    ));
}
