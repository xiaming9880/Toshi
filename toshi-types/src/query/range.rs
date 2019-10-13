use std::ops::Bound;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{to_value, Value};
use tantivy::query::{Query as TantivyQuery, RangeQuery as TantivyRangeQuery};
use tantivy::schema::{FieldType, Schema};

use crate::query::{CreateQuery, KeyValue, Query};
use crate::{error::Error, Result};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Ranges {
    ValueRange {
        gte: Option<Value>,
        lte: Option<Value>,
        lt: Option<Value>,
        gt: Option<Value>,
        boost: Option<f32>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RangeQuery {
    range: KeyValue<String, Ranges>,
}

impl CreateQuery for RangeQuery {
    fn create_query(self, schema: &Schema) -> Result<Box<dyn TantivyQuery>> {
        let KeyValue { field, value, .. } = self.range;
        create_range_query(schema, &field, value)
    }
}

impl RangeQuery {
    pub fn new(field: String, ranges: Ranges) -> Self {
        Self {
            range: KeyValue::new(field, ranges),
        }
    }

    pub fn builder<V>() -> RangeQueryBuilder<V>
    where
        V: Serialize + Default,
    {
        RangeQueryBuilder::default()
    }
}

#[derive(Default)]
pub struct RangeQueryBuilder<V>
where
    V: Serialize + Default,
{
    field: String,
    gte: V,
    lte: V,
    lt: V,
    gt: V,
    boost: f32,
}

impl<V> RangeQueryBuilder<V>
where
    V: Serialize + Default,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_field<F: ToString>(mut self, field: F) -> Self {
        self.field = field.to_string();
        self
    }

    pub fn gte(mut self, gte: V) -> Self {
        self.gte = gte;
        self
    }

    pub fn lte(mut self, lte: V) -> Self {
        self.lte = lte;
        self
    }
    pub fn lt(mut self, lt: V) -> Self {
        self.lt = lt;
        self
    }

    pub fn gt(mut self, gt: V) -> Self {
        self.gt = gt;
        self
    }

    pub fn with_boost(mut self, boost: f32) -> Self {
        self.boost = boost;
        self
    }

    pub fn build(self) -> Query {
        let range_q = Ranges::ValueRange {
            gte: to_value(self.gte).ok(),
            lte: to_value(self.lte).ok(),
            lt: to_value(self.lt).ok(),
            gt: to_value(self.gt).ok(),
            boost: Some(self.boost),
        };
        Query::Range(RangeQuery::new(self.field, range_q))
    }
}

#[inline]
fn include_exclude<V>(r: Option<Value>, r2: Option<Value>) -> Result<Bound<V>>
where
    V: DeserializeOwned,
{
    if let Some(b) = r {
        let value = serde_json::from_value(b).map_err(Error::from)?;
        Ok(Bound::Excluded(value))
    } else if let Some(b) = r2 {
        let value = serde_json::from_value(b).map_err(Error::from)?;
        Ok(Bound::Included(value))
    } else {
        Ok(Bound::Unbounded)
    }
}

#[inline]
fn create_ranges<V>(gte: Option<Value>, lte: Option<Value>, lt: Option<Value>, gt: Option<Value>) -> Result<(Bound<V>, Bound<V>)>
where
    V: DeserializeOwned,
{
    Ok((include_exclude(lt, lte)?, include_exclude(gt, gte)?))
}

pub fn create_range_query(schema: &Schema, field: &str, r: Ranges) -> Result<Box<dyn TantivyQuery>> {
    match r {
        Ranges::ValueRange { gte, lte, lt, gt, .. } => {
            let field = schema
                .get_field(field)
                .ok_or_else(|| Error::QueryError(format!("Field {} does not exist", field)))?;
            let field_type = schema.get_field_entry(field).field_type();
            match field_type {
                &FieldType::I64(_) => {
                    let (upper, lower) = create_ranges::<i64>(gte, lte, lt, gt)?;
                    Ok(Box::new(TantivyRangeQuery::new_i64_bounds(field, lower, upper)))
                }
                &FieldType::U64(_) => {
                    let (upper, lower) = create_ranges::<u64>(gte, lte, lt, gt)?;
                    Ok(Box::new(TantivyRangeQuery::new_u64_bounds(field, lower, upper)))
                }
                ref ft => Err(Error::QueryError(format!("Invalid field type: {:?} for range query", ft))),
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use tantivy::schema::*;

    use super::*;

    #[test]
    pub fn test_deserialize_missing_ranges() {
        let body = r#"{ "range" : { "test_i64" : { "gte" : 2012 } } }"#;
        let req = serde_json::from_str::<RangeQuery>(body);
        assert_eq!(req.is_err(), false);
    }

    #[test]
    pub fn test_query_creation_bad_type() {
        let body = r#"{ "range" : { "test_i64" : { "gte" : 3.14 } } }"#;
        let mut schema = SchemaBuilder::new();
        schema.add_i64_field("test_i64", FAST);
        let built = schema.build();
        let req = serde_json::from_str::<RangeQuery>(body).unwrap().create_query(&built);

        assert_eq!(req.is_err(), true);
        assert_eq!(
            req.unwrap_err().to_string(),
            "Error in query execution: 'invalid type: floating point `3.14`, expected i64'"
        );
    }

    #[test]
    pub fn test_query_creation_bad_range() {
        let body = r#"{ "range" : { "test_u64" : { "gte" : -1 } } }"#;
        let mut schema = SchemaBuilder::new();
        schema.add_u64_field("test_u64", FAST);
        let built = schema.build();
        let req = serde_json::from_str::<RangeQuery>(body).unwrap().create_query(&built);

        assert_eq!(req.is_err(), true);
        assert_eq!(
            req.unwrap_err().to_string(),
            "Error in query execution: 'invalid value: integer `-1`, expected u64'"
        );
    }

    #[test]
    pub fn test_query_impossible_range() {
        let body = r#"{ "range" : { "test_u64" : { "gte" : 10, "lte" : 1 } } }"#;
        let mut schema = SchemaBuilder::new();
        schema.add_u64_field("test_u64", FAST);
        let built = schema.build();
        let req = serde_json::from_str::<RangeQuery>(body).unwrap().create_query(&built);

        assert_eq!(req.is_err(), false);
    }
}
