// Which side of the switch a plan belongs to.
export type Mode = 'Individual' | 'Enterprise'

export type Plan = {
  name: string
  mode: Mode
  price: string
  cadence: string | null
  who: string
  features: string[]
}

// Stands in until a plan lives on the account.
export const current = 'Dev'

export const plans: Plan[] = [
  {
    name: 'Dev',
    mode: 'Individual',
    price: 'Free',
    cadence: null,
    who: 'Enough to see whether it fits.',
    features: [
      '1 app and 1,000 runs a month',
      'Every host function you define',
      'One schedule, hourly',
      'A week of runs and their traces',
      'Rollback to any version'
    ]
  },
  {
    name: 'Sandbox',
    mode: 'Individual',
    price: '$5.49',
    cadence: 'a month',
    who: 'For an agent that writes code and needs to run it.',
    features: [
      'Runs by volume',
      'Snapshots to restore, fork to branch',
      'Five schedules, every 15 minutes',
      'A month of runs, with keyframes to scrub',
      'Support by email'
    ]
  },
  {
    name: 'Team',
    mode: 'Individual',
    price: '$54.9',
    cadence: 'a month',
    who: 'For a product whose customers write the logic.',
    features: [
      'Unlimited apps, tenants by volume',
      'The editor, with your functions autocompleted',
      'Your own keywords, so it reads like your product',
      'Schedules every minute, run by us',
      'A year of runs, with the full trace'
    ]
  },
  {
    name: 'Enterprise',
    mode: 'Enterprise',
    price: 'Talk to us',
    cadence: null,
    who: 'For when you have to prove what ran.',
    features: [
      'A verifiable receipt for every run',
      'A pinned build, signed, with an SBOM',
      'An offline mirror',
      'Single sign-on and an audit log',
      'What you run, insured in dollars'
    ]
  }
]
