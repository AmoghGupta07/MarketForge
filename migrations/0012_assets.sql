-- Buying or selling an asset (rental property, solar panels, a car) swaps
-- cash for something you own, like trades and home purchases: it is not
-- income or spending. Rent, upkeep and mortgage interest on those assets
-- are still income and expenses ('rent_income', 'asset_costs', 'interest').
CREATE OR REPLACE FUNCTION is_transfer(cat TEXT) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT cat IN ('trade', 'loan_draw', 'loan_principal', 'deposit', 'withdrawal', 'deal',
                   'business_invest', 'home', 'mortgage_principal', 'asset')
$$;
