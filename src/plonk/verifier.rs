use super::{hash_point, Proof, SRS};
use crate::arithmetic::{get_challenge_scalar, Challenge, Curve, CurveAffine, Field};
use crate::polycommit::Params;
use crate::transcript::Hasher;

impl<C: CurveAffine> Proof<C> {
    /// Returns
    pub fn verify<HBase: Hasher<C::Base>, HScalar: Hasher<C::Scalar>>(
        &self,
        params: &Params<C>,
        srs: &SRS<C>,
    ) -> bool {
        // Create a transcript for obtaining Fiat-Shamir challenges.
        let mut transcript = HBase::init(C::Base::one());

        for commitment in &self.advice_commitments {
            hash_point(&mut transcript, commitment)
                .expect("proof cannot contain points at infinity");
        }

        let x_2: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        for c in &self.h_commitments {
            hash_point(&mut transcript, c).expect("proof cannot contain points at infinity");
        }

        let x_3: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        let mut transcript_scalar = HScalar::init(C::Scalar::one());

        for eval in self.advice_evals_x.iter() {
            transcript_scalar.absorb(*eval);
        }

        for eval in self.fixed_evals_x.iter() {
            transcript_scalar.absorb(*eval);
        }

        for eval in &self.h_evals_x {
            transcript_scalar.absorb(*eval);
        }

        // Evaluate the circuit using the custom gates provided
        let mut h_eval = C::Scalar::zero();
        for poly in srs.meta.gates.iter() {
            h_eval *= &x_2;

            let evaluation: C::Scalar = poly.evaluate(
                &|index| self.fixed_evals_x[index],
                &|index| self.advice_evals_x[index],
                &|a, b| a + &b,
                &|a, b| a * &b,
                &|a, scalar| a * &scalar,
            );

            h_eval += &evaluation;
        }
        let xn = x_3.pow(&[params.n as u64, 0, 0, 0]);
        h_eval *= &(xn - &C::Scalar::one()).invert().unwrap();

        // Compute the expected h(x) value
        let mut expected_h_eval = C::Scalar::zero();
        let mut cur = C::Scalar::one();
        for eval in &self.h_evals_x {
            expected_h_eval += &(cur * eval);
            cur *= &xn;
        }

        if h_eval != expected_h_eval {
            return false;
        }

        let transcript_scalar_point =
            C::Base::from_bytes(&(transcript_scalar.squeeze()).to_bytes()).unwrap();
        transcript.absorb(transcript_scalar_point);

        let x_4: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        let mut q_commitments: Vec<_> = vec![None; srs.meta.query_rows.len()];
        let mut q_evals: Vec<_> = vec![C::Scalar::zero(); srs.meta.query_rows.len()];

        {
            for (i, &(wire, ref at)) in srs.meta.advice_queries.iter().enumerate() {
                let query_row = *srs.meta.query_rows.get(at).unwrap();

                if q_commitments[query_row].is_none() {
                    q_commitments[query_row] =
                        Some(self.advice_commitments[wire.0].to_projective());
                    q_evals[query_row] = self.advice_evals_x[i];
                } else {
                    q_commitments[query_row].as_mut().map(|commitment| {
                        *commitment *= x_4;
                        *commitment += self.advice_commitments[wire.0];
                    });
                    q_evals[query_row] *= &x_4;
                    q_evals[query_row] += &self.advice_evals_x[i];
                }
            }

            for (i, &(wire, ref at)) in srs.meta.fixed_queries.iter().enumerate() {
                let query_row = *srs.meta.query_rows.get(at).unwrap();

                if q_commitments[query_row].is_none() {
                    q_commitments[query_row] = Some(srs.fixed_commitments[wire.0].to_projective());
                    q_evals[query_row] = self.fixed_evals_x[i];
                } else {
                    q_commitments[query_row].as_mut().map(|commitment| {
                        *commitment *= x_4;
                        *commitment += srs.fixed_commitments[wire.0];
                    });
                    q_evals[query_row] *= &x_4;
                    q_evals[query_row] += &self.fixed_evals_x[i];
                }
            }

            for (h_commitment, h_eval) in self.h_commitments.iter().zip(self.h_evals_x.iter()) {
                // We query the h(X) polynomial at x_3
                let cur_row = *srs.meta.query_rows.get(&0).unwrap();

                if q_commitments[cur_row].is_none() {
                    q_commitments[cur_row] = Some(h_commitment.to_projective());
                    q_evals[cur_row] = *h_eval;
                } else {
                    q_commitments[cur_row].as_mut().map(|commitment| {
                        *commitment *= x_4;
                        *commitment += *h_commitment;
                    });
                    q_evals[cur_row] *= &x_4;
                    q_evals[cur_row] += h_eval;
                }
            }
        }

        let x_5: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        hash_point(&mut transcript, &self.f_commitment)
            .expect("proof cannot contain points at infinity");

        let x_6: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        // We can compute the expected f_eval from x_5
        let mut f_eval = C::Scalar::zero();
        for (&row, &col) in srs.meta.query_rows.iter() {
            let mut eval: C::Scalar = self.q_evals[col].clone();
            let mut point = x_3;
            if row >= 0 {
                point *= &srs.domain.get_omega().pow_vartime(&[row as u64, 0, 0, 0]);
            } else {
                point *= &srs
                    .domain
                    .get_omega_inv()
                    .pow_vartime(&[row.abs() as u64, 0, 0, 0]);
            }
            eval = eval - &q_evals[col];
            eval = eval * &(x_6 - &point).invert().unwrap();

            f_eval *= &x_5;
            f_eval += &eval;
        }

        for eval in self.q_evals.iter() {
            transcript_scalar.absorb(*eval);
        }

        let transcript_scalar_point =
            C::Base::from_bytes(&(transcript_scalar.squeeze()).to_bytes()).unwrap();
        transcript.absorb(transcript_scalar_point);

        let x_7: C::Scalar = get_challenge_scalar(Challenge(transcript.squeeze().get_lower_128()));

        let mut f_commitment: C::Projective = self.f_commitment.to_projective();
        for (_, &col) in srs.meta.query_rows.iter() {
            f_commitment *= x_7;
            f_commitment = f_commitment + &q_commitments[col].as_ref().unwrap();
            f_eval *= &x_7;
            f_eval += &self.q_evals[col];
        }

        params.verify_proof(
            &self.opening,
            &mut transcript,
            x_6,
            &f_commitment.to_affine(),
            f_eval,
        )
    }
}
