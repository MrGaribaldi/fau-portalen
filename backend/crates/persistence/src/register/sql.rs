//! Shared pieces of the register's SQL: the code columns' mapping onto the domain's enums.

use fau_domain::register::sync::{
    MunicipalitySource, MunicipalityStatus, Origin, Ownership, SchoolStatus, Verification,
};

use super::error::RegisterError;

pub(crate) fn municipality_status(code: &str) -> Result<MunicipalityStatus, RegisterError> {
    match code {
        "active" => Ok(MunicipalityStatus::Active),
        "dissolved" => Ok(MunicipalityStatus::Dissolved),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn municipality_source(code: &str) -> Result<MunicipalitySource, RegisterError> {
    match code {
        "kartverket" => Ok(MunicipalitySource::Kartverket),
        "manual" => Ok(MunicipalitySource::Manual),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn origin(code: &str) -> Result<Origin, RegisterError> {
    match code {
        "register" => Ok(Origin::Register),
        "submitted" => Ok(Origin::Submitted),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn verification(code: &str) -> Result<Verification, RegisterError> {
    match code {
        "listed" => Ok(Verification::Listed),
        "pending" => Ok(Verification::Pending),
        "verified" => Ok(Verification::Verified),
        "rejected" => Ok(Verification::Rejected),
        "held" => Ok(Verification::Held),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn school_status(code: &str) -> Result<SchoolStatus, RegisterError> {
    match code {
        "active" => Ok(SchoolStatus::Active),
        "closed" => Ok(SchoolStatus::Closed),
        _ => Err(RegisterError::Decode),
    }
}

pub(crate) fn ownership(code: Option<&str>) -> Result<Option<Ownership>, RegisterError> {
    match code {
        None => Ok(None),
        Some("public") => Ok(Some(Ownership::Public)),
        Some("private") => Ok(Some(Ownership::Private)),
        Some(_) => Err(RegisterError::Decode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_decodes_and_an_unknown_one_does_not() {
        assert_eq!(verification("held"), Ok(Verification::Held));
        assert_eq!(verification("Held"), Err(RegisterError::Decode));
        assert_eq!(ownership(None), Ok(None));
        assert_eq!(ownership(Some("private")), Ok(Some(Ownership::Private)));
        assert_eq!(
            municipality_source("manual"),
            Ok(MunicipalitySource::Manual)
        );
        assert_eq!(school_status("closed"), Ok(SchoolStatus::Closed));
        assert_eq!(origin("submitted"), Ok(Origin::Submitted));
        assert_eq!(
            municipality_status("dissolved"),
            Ok(MunicipalityStatus::Dissolved)
        );
    }
}
