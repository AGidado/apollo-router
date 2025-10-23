mod satisfiability;

use std::vec;

pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
pub use crate::schema::schema_upgrader::upgrade_subgraphs_if_necessary;
use crate::subgraph::typestate::Expanded;
use crate::subgraph::typestate::Initial;
use crate::subgraph::typestate::Subgraph;
use crate::subgraph::typestate::Upgraded;
use crate::subgraph::typestate::Validated;
pub use crate::supergraph::Merged;
pub use crate::supergraph::Satisfiable;
pub use crate::supergraph::Supergraph;

pub fn compose(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    let expanded_subgraphs = expand_subgraphs(subgraphs)?;
    let upgraded_subgraphs = upgrade_subgraphs_if_necessary(expanded_subgraphs)?;
    let validated_subgraphs = validate_subgraphs(upgraded_subgraphs)?;

    pre_merge_validations(&validated_subgraphs)?;
    let supergraph = merge_subgraphs(validated_subgraphs)?;
    post_merge_validations(&supergraph)?;
    validate_satisfiability(supergraph)
}

/// Apollo Federation allow subgraphs to specify partial schemas (i.e. "import" directives through
/// `@link`). This function will update subgraph schemas with all missing federation definitions.
pub fn expand_subgraphs(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Vec<Subgraph<Expanded>>, Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];
    let expanded: Vec<Subgraph<Expanded>> = subgraphs
        .into_iter()
        .map(|s| s.expand_links())
        .filter_map(|r| r.map_err(|e| errors.push(e.into())).ok())
        .collect();
    if errors.is_empty() {
        Ok(expanded)
    } else {
        Err(errors)
    }
}

/// Validate subgraph schemas to ensure they satisfy Apollo Federation requirements (e.g. whether
/// `@key` specifies valid `FieldSet`s etc).
pub fn validate_subgraphs(
    subgraphs: Vec<Subgraph<Upgraded>>,
) -> Result<Vec<Subgraph<Validated>>, Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];
    let validated: Vec<Subgraph<Validated>> = subgraphs
        .into_iter()
        .map(|s| s.validate())
        .filter_map(|r| r.map_err(|e| errors.push(e.into())).ok())
        .collect();
    if errors.is_empty() {
        Ok(validated)
    } else {
        Err(errors)
    }
}

/// Perform validations that require information about all available subgraphs.
pub fn pre_merge_validations(
    subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    let mut errors = Vec::new();
    
    // Validate that we have at least one subgraph
    if subgraphs.is_empty() {
        errors.push(CompositionError::InternalError {
            message: "Cannot compose with no subgraphs".to_string(),
        });
        return Err(errors);
    }
    
    // Validate that all subgraphs have unique names
    let mut seen_names = std::collections::HashSet::new();
    for subgraph in subgraphs {
        if !seen_names.insert(&subgraph.name) {
            errors.push(CompositionError::InternalError {
                message: format!("Duplicate subgraph name: {}", subgraph.name),
            });
        }
    }
    
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn merge_subgraphs(
    subgraphs: Vec<Subgraph<Validated>>,
) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
    use crate::merge::merge_subgraphs as merge_fn;
    use crate::subgraph::ValidSubgraph;
    
    // Convert Subgraph<Validated> to ValidSubgraph
    let valid_subgraphs: Vec<ValidSubgraph> = subgraphs
        .into_iter()
        .map(|subgraph| ValidSubgraph {
            name: subgraph.name.clone(),
            url: subgraph.url.clone(),
            schema: apollo_compiler::validation::Valid::assume_valid(subgraph.schema().schema().clone()),
        })
        .collect();
    
    // Use the existing merge function
    let subgraph_refs: Vec<&ValidSubgraph> = valid_subgraphs.iter().collect();
    let merge_result = merge_fn(subgraph_refs)
        .map_err(|failure| {
            failure.errors.into_iter()
                .map(|err| CompositionError::InternalError { message: err })
                .collect::<Vec<CompositionError>>()
        })?;
    
    // Create a Supergraph<Merged> from the merged schema
    let supergraph = Supergraph::<Merged>::new(merge_result.schema);
    Ok(supergraph)
}

pub fn post_merge_validations(
    supergraph: &Supergraph<Merged>,
) -> Result<(), Vec<CompositionError>> {
    let mut errors = Vec::new();
    
    // Validate that the supergraph has a query root type
    let schema = supergraph.schema();
    if schema.root_operation(apollo_compiler::ast::OperationType::Query).is_none() {
        errors.push(CompositionError::InternalError {
            message: "No queries found in any subgraph: a supergraph must have a query root type".to_string(),
        });
    }
    
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
