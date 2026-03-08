Pilot = @(name: Str, age: Int, role: Str)

pilotMember <= Pilot(name <= _, age <= 14, role <= "pilot")
rei <= pilotMember("Rei")
stdout(rei.name)
stdout(rei.age.toString())
stdout(rei.role)
