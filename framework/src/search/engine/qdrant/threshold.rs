//! Threshold retrieval bounds ordinary multi-attribute BM25 queries while
//! retaining exact sums and complete boundary ties. Qdrant computes every score.
use super::*;
use std::collections::HashSet;

impl QdrantSearchEngine {
    pub(super) async fn complete_attribute_scores(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        settings: &IndexSettings,
        prefetch: &mut qb::PrefetchQuery,
    ) -> Result<Option<Vec<EngineCandidate>>, FrameworkError> {
        let attributes = prefetch
            .prefetch
            .iter()
            .filter(|p| {
                p.using
                    .as_deref()
                    .is_some_and(|name| name.starts_with("bm25_"))
            })
            .cloned()
            .collect::<Vec<_>>();
        if attributes.len() <= 1 {
            return Ok(None);
        }
        let depth = usize::try_from(prefetch.limit.unwrap_or(0))
            .map_err(|_| error(collection, "candidate depth exceeds addressable size"))?;
        if depth == 0 {
            return Ok(Some(vec![]));
        }
        let weights = settings
            .index_time
            .searchable_attributes
            .iter()
            .map(|attr| attr.weight)
            .collect::<Vec<_>>();
        let mut window = depth as u64;
        let (mut candidates, complete, ids) = loop {
            let (ids, threshold, exhausted) = self
                .attribute_union(collection, &attributes, &weights, window)
                .await?;
            let complete = attributes
                .iter()
                .map(|attribute| {
                    let mut child = attribute.clone();
                    child.filter = Some(QFilter::must([
                        child.filter.take().unwrap_or_default().into(),
                        Condition::has_id(ids.iter().cloned()),
                    ]));
                    child.limit = Some((ids.len() as u64).max(1));
                    child
                })
                .collect::<Vec<_>>();
            let candidates = self
                .rescore_attributes(index, collection, &complete, &weights, ids.len())
                .await?;
            // Equality continues: an unseen point can share the boundary score
            // and precede it by document key. No attribute contribution is lost.
            if exhausted
                || candidates
                    .get(depth - 1)
                    .is_some_and(|c| c.score > f64::from(threshold))
            {
                break (candidates, complete, ids);
            }
            window = window
                .checked_mul(2)
                .ok_or_else(|| error(collection, "BM25 threshold window overflow"))?;
        };
        if candidates.get(depth - 1).is_none_or(|c| c.score == 0.0) {
            let zeros = self
                .zero_score_candidates(index, collection, prefetch, &ids, depth)
                .await?;
            candidates.extend(zeros);
        }
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.document.key.cmp(&b.document.key))
        });
        candidates.truncate(depth);
        // Freeze the exact lexical top-k for the hybrid leg. Its OrderBy child
        // stays at depth and contributes only candidates, never formula scores.
        prefetch.prefetch[..complete.len()].clone_from_slice(&complete);
        prefetch.filter = Some(QFilter::must([
            prefetch.filter.take().unwrap_or_default().into(),
            Condition::has_id(
                candidates
                    .iter()
                    .map(|c| QdrantVectorDriver::resolve_point_id(&c.document.key)),
            ),
        ]));
        if let Some(zeros) = prefetch.prefetch.last_mut() {
            zeros.filter = prefetch.filter.clone();
        }
        Ok(Some(candidates))
    }

    async fn attribute_union(
        &self,
        collection: &str,
        attributes: &[qb::PrefetchQuery],
        weights: &[f32],
        window: u64,
    ) -> Result<(HashSet<qb::PointId>, f32, bool), FrameworkError> {
        let mut ids = HashSet::new();
        // Qdrant evaluates formulas in f64 and casts the final score to f32.
        // Match that rounding so a tied upper bound cannot look strictly lower.
        let mut threshold = 0.0_f64;
        let mut exhausted = true;
        for (attribute, weight) in attributes.iter().zip(weights) {
            let request = qb::QueryPoints {
                collection_name: collection.into(),
                query: attribute.query.clone(),
                using: attribute.using.clone(),
                filter: attribute.filter.clone(),
                params: attribute.params.clone(),
                limit: Some(window),
                ..Default::default()
            };
            let points = self
                .ranked_query(request)
                .await
                .map_err(|e| error(collection, e))?
                .result;
            if points.len() as u64 == window {
                exhausted = false;
                threshold += f64::from(*weight) * f64::from(points.last().map_or(0.0, |p| p.score));
            }
            for point in points {
                ids.insert(
                    point
                        .id
                        .ok_or_else(|| error(collection, "BM25 point is missing its id"))?,
                );
            }
        }
        Ok((ids, threshold as f32, exhausted))
    }

    async fn rescore_attributes(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        attributes: &[qb::PrefetchQuery],
        weights: &[f32],
        count: usize,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        if count == 0 {
            return Ok(vec![]);
        }
        let response = self
            .ranked_query(
                qb::QueryPointsBuilder::new(collection)
                    .prefetch(attributes.to_vec())
                    .query(score_formula(weights))
                    .limit(count as u64)
                    .with_payload(hit_payload(index))
                    .build(),
            )
            .await
            .map_err(|e| error(collection, e))?;
        decode_scored(index, response.result)
    }

    async fn zero_score_candidates(
        &self,
        index: &IndexDescriptor,
        collection: &str,
        prefetch: &qb::PrefetchQuery,
        positive_ids: &HashSet<qb::PointId>,
        depth: usize,
    ) -> Result<Vec<EngineCandidate>, FrameworkError> {
        let mut filter = prefetch.filter.clone().unwrap_or_default();
        filter
            .must_not
            .push(Condition::has_id(positive_ids.iter().cloned()));
        let order = prefetch
            .prefetch
            .last()
            .and_then(|p| p.query.clone())
            .ok_or_else(|| error(collection, "BM25 prefetch is missing its zero-score query"))?;
        let response = self
            .ranked_query(
                qb::QueryPointsBuilder::new(collection)
                    .query(order)
                    .filter(filter.clone())
                    .limit(depth as u64)
                    .with_payload(hit_payload(index))
                    .build(),
            )
            .await
            .map_err(|e| error(collection, e))?;
        let mut zeros = decode_scored(index, response.result)?;
        // OrderBy exposes its ordering value as score. It is only a candidate
        // source; lexical matches outside every attribute score exactly zero.
        for zero in &mut zeros {
            zero.score = 0.0;
        }
        if zeros.len() == depth {
            // All zero scores tie. Resolve their document keys in bounded
            // scroll pages, without growing either attribute or OrderBy limits.
            zeros.clear();
            let mut offset = None;
            loop {
                let page = self
                    .scroll_page(collection, filter.clone(), offset, None)
                    .await?;
                for point in page.result {
                    zeros.push(EngineCandidate {
                        document: decode_document(index, point.payload)?,
                        score: 0.0,
                        highlights: None,
                    });
                }
                zeros.sort_by(|a, b| a.document.key.cmp(&b.document.key));
                zeros.truncate(depth);
                offset = page.next_page_offset;
                if offset.is_none() {
                    break;
                }
            }
        }
        Ok(zeros)
    }
}
